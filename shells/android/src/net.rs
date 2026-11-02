//! HTTP for Android: each request runs on a thread of its own through
//! Java's `HttpURLConnection` (the system's TLS and certificates, no extra
//! library), and the event loop is woken when an answer has arrived.
//! Redirects are followed here, not by Java, as `paperpaw_http::follow`
//! says for every shell: never down to plain `http`, never with
//! credentials to another server.

use std::cell::RefCell;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use android_activity::{AndroidApp, AndroidAppWaker};
use jni::JNIEnv;
use jni::JavaVM;
use jni::objects::{JObject, JValue};
use paperpaw_core::sdk::HttpError;
use paperpaw_core::{HttpCall, HttpResponse, MAX_RESPONSE};

/// How much of an answer's body came: the request's id, the bytes so
/// far, and in all where the answer said.
type Progress = (u32, u64, Option<u64>);

/// Requests in flight and their answers.
pub struct Net {
    vm: usize,
    waker: AndroidAppWaker,
    sender: Sender<HttpResponse>,
    receiver: Receiver<HttpResponse>,
    /// How far the answers that ask for it have come.
    progress: (Sender<Progress>, Receiver<Progress>),
}

/// Where a request's thread tells how far its answer came: set on the
/// thread of a request that asks ([`HttpCall::progress`]).
struct Teller {
    sender: Sender<Progress>,
    waker: AndroidAppWaker,
    /// The bytes last told: the next once a hundredth further on (or
    /// 256 KiB where the size is not known), so the loop is woken seldom.
    told: Option<u64>,
}

thread_local! {
    // Already `const`: clippy (1.99) asks for it all the same, without a
    // suggestion to apply.
    #[allow(clippy::missing_const_for_thread_local)]
    static TELLER: RefCell<Option<Teller>> = const { RefCell::new(None) };
}

/// Tells how much of request `id`'s body came, where its thread tells.
fn tell(id: u32, received: u64, total: Option<u64>) {
    TELLER.with_borrow_mut(|teller| {
        let Some(teller) = teller.as_mut() else {
            return;
        };
        let step = total.map_or(256 << 10, |total| (total / 100).max(1));
        if teller
            .told
            .is_some_and(|told| received < told.saturating_add(step))
        {
            return;
        }
        teller.told = Some(received);
        let _ = teller.sender.send((id, received, total));
        teller.waker.wake();
    });
}

impl Net {
    pub fn new(app: &AndroidApp) -> Self {
        let (sender, receiver) = channel();
        Self {
            vm: app.vm_as_ptr() as usize,
            waker: app.create_waker(),
            sender,
            receiver,
            progress: channel(),
        }
    }

    /// Starts a request.
    pub fn start(&self, call: HttpCall) {
        let (vm, waker, sender) = (self.vm, self.waker.clone(), self.sender.clone());
        let teller = call.progress.then(|| Teller {
            sender: self.progress.0.clone(),
            waker: self.waker.clone(),
            told: None,
        });
        thread::spawn(move || {
            TELLER.with_borrow_mut(|told| *told = teller);
            // SAFETY: the process's Java VM, valid while the app runs.
            let response = match unsafe { JavaVM::from_raw(vm as *mut jni::sys::JavaVM) } {
                Ok(vm) => perform(&vm, call),
                Err(_) => failed(&call, HttpError::Io, "no Java VM"),
            };
            let _ = sender.send(response);
            waker.wake();
        });
    }

    /// Answers a request without making it (e.g. its file to send is
    /// missing), with the others.
    pub fn answer(&self, response: HttpResponse) {
        let _ = self.sender.send(response);
        self.waker.wake();
    }

    /// An answer that has arrived, if any.
    pub fn next(&self) -> Option<HttpResponse> {
        self.receiver.try_recv().ok()
    }

    /// How far an answer that asks for it has come, if it did since.
    pub fn next_progress(&self) -> Option<Progress> {
        self.progress.1.try_recv().ok()
    }
}

fn failed(call: &HttpCall, error: HttpError, reason: &str) -> HttpResponse {
    HttpResponse::failed(call.id, error, reason)
}

/// Why a request failed, from the Java exception it threw.
fn exception_error(env: &mut JNIEnv<'_>, exception: &JObject<'_>) -> HttpError {
    let kinds = [
        ("java/net/UnknownHostException", HttpError::Dns),
        ("java/net/SocketTimeoutException", HttpError::Timeout),
        ("javax/net/ssl/SSLException", HttpError::Tls),
        ("java/net/ConnectException", HttpError::Refused),
        ("java/net/NoRouteToHostException", HttpError::NoNetwork),
    ];
    kinds
        .into_iter()
        .find(|(class, _)| env.is_instance_of(exception, *class).unwrap_or(false))
        .map_or(HttpError::Io, |(_, error)| error)
}

/// Redirects followed at most.
const MAX_REDIRECTS: u32 = 10;

fn perform(vm: &JavaVM, mut call: HttpCall) -> HttpResponse {
    let Ok(mut env) = vm.attach_current_thread() else {
        return failed(&call, HttpError::Io, "cannot attach to the Java VM");
    };
    match request(&mut env, &mut call) {
        Ok(response) => response,
        Err(e) => {
            // E.g. no network or an unknown host (an IOException).
            let mut error = HttpError::Io;
            if env.exception_check().unwrap_or(false) {
                let exception = env.exception_occurred();
                let _ = env.exception_clear();
                if let Ok(exception) = exception {
                    error = exception_error(&mut env, &exception);
                }
            }
            // The host only: addresses may carry keys.
            let host = paperpaw_http::Url::parse(&call.url).map_or("?", |url| url.host);
            log::warn!("{} {host}: {e}", call.method);
            failed(&call, error, "request failed")
        }
    }
}

/// A header of the answer of `conn`, if it has it.
fn header(
    env: &mut JNIEnv<'_>,
    conn: &JObject<'_>,
    name: &str,
) -> jni::errors::Result<Option<String>> {
    let name = env.new_string(name)?;
    let value = env
        .call_method(
            conn,
            "getHeaderField",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JValue::Object(&name)],
        )?
        .l()?;
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(String::from(
        env.get_string(&jni::objects::JString::from(value))?,
    )))
}

/// What one exchange with a server came to.
enum Step {
    Answer(HttpResponse),
    /// A redirect with this status, to this `Location`.
    Redirect(u16, String),
}

/// Makes the request, following redirects; `call` changes on the way as
/// `paperpaw_http::follow` says (its method, body and headers).
fn request(env: &mut JNIEnv<'_>, call: &mut HttpCall) -> jni::errors::Result<HttpResponse> {
    let mut url = call.url.clone();
    for _ in 0..=MAX_REDIRECTS {
        // Each exchange's Java objects go with it.
        let step = env.with_local_frame(32, |env| exchange(env, &url, call))?;
        let (status, location) = match step {
            Step::Answer(mut response) => {
                if url != call.url {
                    response.url = Some(url);
                }
                return Ok(response);
            }
            Step::Redirect(status, location) => (status, location),
        };
        let body = call.body.is_some();
        let redirect = match paperpaw_http::follow(&url, status, &location, &call.method, body) {
            Ok(redirect) => redirect,
            Err(refusal) => return Ok(failed(call, HttpError::Refused, refusal.reason())),
        };
        if redirect.to_get {
            call.method = String::from("GET");
            call.body = None;
        }
        redirect.keep(&mut call.headers);
        url = redirect.url;
    }
    Ok(failed(call, HttpError::Refused, "too many redirects"))
}

/// Sends the request to `url` and takes the answer, or where it redirects.
fn exchange(env: &mut JNIEnv<'_>, url: &str, call: &HttpCall) -> jni::errors::Result<Step> {
    let conn = connect(env, url, call)?;
    let status = env.call_method(&conn, "getResponseCode", "()I", &[])?.i()?;
    let status = status.clamp(0, i32::from(u16::MAX)) as u16;
    if matches!(status, 301 | 302 | 303 | 307 | 308)
        && let Some(location) = header(env, &conn, "Location")?
    {
        env.call_method(&conn, "disconnect", "()V", &[])?;
        return Ok(Step::Redirect(status, location));
    }
    answer(env, &conn, call, status).map(Step::Answer)
}

/// Opens `url` and sends the request; redirects are left to the caller.
fn connect<'a>(
    env: &mut JNIEnv<'a>,
    url: &str,
    call: &HttpCall,
) -> jni::errors::Result<JObject<'a>> {
    let address = env.new_string(url)?;
    let url = env.new_object(
        "java/net/URL",
        "(Ljava/lang/String;)V",
        &[JValue::Object(&address)],
    )?;
    let conn = env
        .call_method(&url, "openConnection", "()Ljava/net/URLConnection;", &[])?
        .l()?;
    env.call_method(
        &conn,
        "setInstanceFollowRedirects",
        "(Z)V",
        &[JValue::Bool(0)],
    )?;
    set_method(env, &conn, &call.method)?;
    // Java limits connecting and each read, not the whole: both to the
    // call's time, connecting to 15 s at most.
    let limit = i32::try_from(call.timeout_ms).unwrap_or(i32::MAX).max(1);
    env.call_method(
        &conn,
        "setConnectTimeout",
        "(I)V",
        &[JValue::Int(limit.min(15_000))],
    )?;
    env.call_method(&conn, "setReadTimeout", "(I)V", &[JValue::Int(limit)])?;
    for (name, value) in &call.headers {
        let (name, value) = (env.new_string(name)?, env.new_string(value)?);
        env.call_method(
            &conn,
            "setRequestProperty",
            "(Ljava/lang/String;Ljava/lang/String;)V",
            &[JValue::Object(&name), JValue::Object(&value)],
        )?;
    }
    if let Some(body) = &call.body {
        env.call_method(&conn, "setDoOutput", "(Z)V", &[JValue::Bool(1)])?;
        let out = env
            .call_method(&conn, "getOutputStream", "()Ljava/io/OutputStream;", &[])?
            .l()?;
        let bytes: JObject<'_> = env.byte_array_from_slice(body)?.into();
        env.call_method(&out, "write", "([B)V", &[JValue::Object(&bytes)])?;
        env.call_method(&out, "close", "()V", &[])?;
    }
    Ok(conn)
}

/// The methods `HttpURLConnection` takes as they are.
const STANDARD_METHODS: [&str; 7] = ["GET", "POST", "HEAD", "OPTIONS", "PUT", "DELETE", "TRACE"];

/// Sets the request method. Android refuses others than the standard
/// ones (e.g. WebDAV's `PROPFIND`), so those are written into the
/// connection's `method` field directly, and into the connection it
/// hands HTTPS requests to.
fn set_method(env: &mut JNIEnv<'_>, conn: &JObject<'_>, method: &str) -> jni::errors::Result<()> {
    let standard = STANDARD_METHODS.contains(&method);
    let name = env.new_string(if standard { method } else { "POST" })?;
    env.call_method(
        conn,
        "setRequestMethod",
        "(Ljava/lang/String;)V",
        &[JValue::Object(&name)],
    )?;
    if standard {
        return Ok(());
    }
    let value = env.new_string(method)?;
    let field = |env: &mut JNIEnv<'_>, target: &JObject<'_>| {
        let class = env.find_class("java/net/HttpURLConnection")?;
        let id = env.get_field_id(&class, "method", "Ljava/lang/String;")?;
        env.set_field_unchecked(target, id, JValue::Object(&value))
    };
    field(env, conn)?;
    // HTTPS connections delegate to another one.
    let class = env.get_object_class(conn)?;
    match env.get_field_id(&class, "delegate", "Ljava/net/HttpURLConnection;") {
        Ok(id) => {
            let delegate = env
                .get_field_unchecked(conn, id, jni::signature::ReturnType::Object)?
                .l()?;
            if !delegate.is_null() {
                field(env, &delegate)?;
            }
        }
        Err(_) => {
            let _ = env.exception_clear();
        }
    }
    Ok(())
}

/// Reads the answer of `conn`: its body cut at `max_response`, or, for a
/// file (`save_to`), failed if longer. An error's body is its reason, read
/// up to [`MAX_RESPONSE`] also for a file.
fn answer(
    env: &mut JNIEnv<'_>,
    conn: &JObject<'_>,
    call: &HttpCall,
    status: u16,
) -> jni::errors::Result<HttpResponse> {
    let success = (200..300).contains(&status);
    let (max, whole) = match &call.save_to {
        Some(_) if success => (call.max_response, true),
        Some(_) => (call.max_response.min(MAX_RESPONSE), false),
        None => (call.max_response, false),
    };
    let etag = header(env, conn, "ETag")?;
    let content_type = header(env, conn, "Content-Type")?;
    // The size, for the progress of a successful answer (-1: not said).
    let total = if success && call.progress {
        let length = env
            .call_method(conn, "getContentLengthLong", "()J", &[])?
            .j()?;
        u64::try_from(length).ok()
    } else {
        None
    };
    let mut headers = Vec::new();
    for name in &call.want_headers {
        if let Some(value) = header(env, conn, name)? {
            headers.push((name.to_ascii_lowercase(), value));
        }
    }
    // Errors come with a body too, e.g. the server's reason.
    let stream = if status >= 400 {
        "getErrorStream"
    } else {
        "getInputStream"
    };
    let stream = env
        .call_method(conn, stream, "()Ljava/io/InputStream;", &[])?
        .l()?;
    let mut body = Vec::new();
    let mut too_long = false;
    if !stream.is_null() {
        let buffer = env.new_byte_array(8192)?;
        loop {
            let n = env
                .call_method(&stream, "read", "([B)I", &[JValue::Object(&buffer)])?
                .i()?;
            let Ok(n) = usize::try_from(n) else {
                break;
            };
            let room = max - body.len();
            if n > room {
                too_long = true;
            }
            let mut chunk = vec![0i8; n.min(room)];
            env.get_byte_array_region(&buffer, 0, &mut chunk)?;
            body.extend(chunk.iter().map(|&b| b as u8));
            if success && call.progress {
                tell(call.id, body.len() as u64, total);
            }
            if too_long {
                break;
            }
        }
        env.call_method(&stream, "close", "()V", &[])?;
    }
    env.call_method(conn, "disconnect", "()V", &[])?;
    if too_long && whole {
        return Ok(failed(call, HttpError::Refused, "too large"));
    }
    Ok(HttpResponse {
        id: call.id,
        status,
        body,
        etag,
        content_type,
        headers,
        truncated: too_long,
        ..HttpResponse::default()
    })
}

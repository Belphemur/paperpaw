//! What the system covers of the window (see `screen`): the status bar, the
//! navigation or gesture bar, a display cutout, as the window's root insets
//! say them. Asked of the activity's window when the window changes (the
//! first start, turning, a split screen); nothing is asked on a timer.

use android_activity::AndroidApp;
use jni::objects::JObject;

use crate::screen::Insets;

/// `Build.VERSION_CODES.R`: where `WindowInsets.getInsets(int)` and the
/// types of insets begin.
const ANDROID_11: i32 = 30;

/// The insets of the window now; none where the system does not say (no
/// window yet, an old system, an exception it leaves).
pub fn read(app: &AndroidApp) -> Insets {
    crate::access::with_activity(app, |env, activity| {
        let window = env
            .call_method(activity, "getWindow", "()Landroid/view/Window;", &[])?
            .l()?;
        let decor = env
            .call_method(&window, "getDecorView", "()Landroid/view/View;", &[])?
            .l()?;
        let insets = env
            .call_method(
                &decor,
                "getRootWindowInsets",
                "()Landroid/view/WindowInsets;",
                &[],
            )?
            .l()?;
        if insets.is_null() {
            return Ok(Insets::default());
        }
        let sdk = env
            .get_static_field("android/os/Build$VERSION", "SDK_INT", "I")?
            .i()?;
        let (left, top, right, bottom) = if sdk >= ANDROID_11 {
            // The bars and the cutout, whichever is larger on each side.
            let types = "android/view/WindowInsets$Type";
            let bars = env
                .call_static_method(types, "systemBars", "()I", &[])?
                .i()?;
            let cutout = env
                .call_static_method(types, "displayCutout", "()I", &[])?
                .i()?;
            let found = env
                .call_method(
                    &insets,
                    "getInsets",
                    "(I)Landroid/graphics/Insets;",
                    &[(bars | cutout).into()],
                )?
                .l()?;
            let side = |env: &mut jni::JNIEnv<'_>, found: &JObject<'_>, name: &str| {
                env.get_field(found, name, "I")?.i()
            };
            (
                side(env, &found, "left")?,
                side(env, &found, "top")?,
                side(env, &found, "right")?,
                side(env, &found, "bottom")?,
            )
        } else {
            let side = |env: &mut jni::JNIEnv<'_>, name: &str| {
                env.call_method(&insets, name, "()I", &[])?.i()
            };
            (
                side(env, "getSystemWindowInsetLeft")?,
                side(env, "getSystemWindowInsetTop")?,
                side(env, "getSystemWindowInsetRight")?,
                side(env, "getSystemWindowInsetBottom")?,
            )
        };
        let pixels = |value: i32| u32::try_from(value).unwrap_or(0);
        Ok(Insets {
            left: pixels(left),
            top: pixels(top),
            right: pixels(right),
            bottom: pixels(bottom),
        })
    })
    .unwrap_or_default()
}

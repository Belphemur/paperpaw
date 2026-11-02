//! The root certificates TLS trusts: Mozilla's, as `webpki-roots` keeps
//! them (each one's name and public key), in flash. They are never all
//! parsed into memory, as a list of trusted certificates for mbedTLS
//! would be (well over 100 KB): mbedTLS checks the chain a server sends,
//! and when its top is not signed by a certificate it knows, [`is_root`]
//! and [`signed`] look it up here: a root itself, or signed by one (found
//! by name, the signature checked with its key), as ESP-IDF's certificate
//! bundle does.
//!
//! Name constraints and path-length limits of roots are not applied:
//! mbedTLS is given no roots (see `tls::Contexts::no_roots`), so it never
//! sees them, and a root here only vouches for the certificate it signed.
//! (None of Mozilla's TLS roots has a name constraint at present, and a few
//! carry path-length limits. To apply them, the roots would have to be
//! given to `mbedtls_ssl_conf_ca_chain`, which costs memory for all of
//! them or a loader for the one found.)

use alloc::vec::Vec;

use mbedtls_rs::sys::{
    mbedtls_md, mbedtls_md_get_size, mbedtls_md_info_from_type, mbedtls_pk_context,
    mbedtls_pk_free, mbedtls_pk_init, mbedtls_pk_parse_public_key, mbedtls_pk_verify_ext,
    mbedtls_x509_crt,
};
use webpki_roots::TLS_SERVER_ROOTS;

/// The contents of a DER `SEQUENCE` (its tag and length taken off), if
/// `der` is exactly one.
fn sequence_contents(der: &[u8]) -> Option<&[u8]> {
    let (&tag, rest) = der.split_first()?;
    if tag != 0x30 {
        return None;
    }
    let (&first, rest) = rest.split_first()?;
    let (len, rest) = if first < 0x80 {
        (usize::from(first), rest)
    } else {
        let bytes = usize::from(first & 0x7f);
        if bytes == 0 || bytes > 4 || rest.len() < bytes {
            return None;
        }
        let (len, rest) = rest.split_at(bytes);
        (
            len.iter().fold(0usize, |n, &b| n << 8 | usize::from(b)),
            rest,
        )
    };
    (rest.len() == len).then_some(rest)
}

/// `contents` as a DER `SEQUENCE`.
fn sequence(contents: &[u8]) -> Vec<u8> {
    let len = contents.len();
    let mut der = Vec::with_capacity(len + 4);
    der.push(0x30);
    if len < 0x80 {
        der.push(len as u8);
    } else if len < 0x100 {
        der.extend_from_slice(&[0x81, len as u8]);
    } else {
        der.extend_from_slice(&[0x82, (len >> 8) as u8, len as u8]);
    }
    der.extend_from_slice(contents);
    der
}

/// The bytes of an mbedTLS buffer.
///
/// # Safety
/// `p` and `len` must describe memory that lives as long as `'a`.
unsafe fn bytes<'a>(p: *const u8, len: usize) -> &'a [u8] {
    if p.is_null() {
        &[]
    } else {
        // SAFETY: as promised by the caller.
        unsafe { core::slice::from_raw_parts(p, len) }
    }
}

/// Whether `crt` is one of the roots itself: its name and key are a
/// root's (whoever else signed it).
///
/// # Safety
/// As for [`signed`].
pub unsafe fn is_root(crt: &mbedtls_x509_crt) -> bool {
    // SAFETY: mbedTLS's buffers of the certificate, alive while it is.
    let (subject, key) = unsafe {
        (
            bytes(crt.subject_raw.p, crt.subject_raw.len),
            bytes(crt.pk_raw.p, crt.pk_raw.len),
        )
    };
    let (Some(subject), Some(key)) = (sequence_contents(subject), sequence_contents(key)) else {
        return false;
    };
    TLS_SERVER_ROOTS
        .iter()
        .any(|root| *root.subject == *subject && *root.subject_public_key_info == *key)
}

/// Whether `crt` is signed by one of the roots: one named as its issuer,
/// whose key checks its signature.
///
/// # Safety
/// `crt` must be a certificate mbedTLS parsed (as given to a verify
/// callback).
pub unsafe fn signed(crt: &mbedtls_x509_crt) -> bool {
    // SAFETY: mbedTLS's buffers of the certificate, alive while it is.
    let (issuer, tbs, signature) = unsafe {
        (
            bytes(crt.issuer_raw.p, crt.issuer_raw.len),
            bytes(crt.tbs.p, crt.tbs.len),
            bytes(crt.private_sig.p, crt.private_sig.len),
        )
    };
    let Some(issuer) = sequence_contents(issuer) else {
        return false;
    };
    // SAFETY: a lookup in mbedTLS's static table.
    let md = unsafe { mbedtls_md_info_from_type(crt.private_sig_md) };
    if md.is_null() {
        return false;
    }
    let mut hash = [0u8; 64];
    // SAFETY: `md` is valid; the hash fits (64 bytes is SHA-512's size).
    let hash_len = usize::from(unsafe { mbedtls_md_get_size(md) });
    if hash_len > hash.len()
        || unsafe { mbedtls_md(md, tbs.as_ptr(), tbs.len(), hash.as_mut_ptr()) } != 0
    {
        return false;
    }
    TLS_SERVER_ROOTS
        .iter()
        .filter(|root| *root.subject == *issuer)
        .any(|root| {
            let key = sequence(&root.subject_public_key_info);
            // SAFETY: an mbedTLS key context, set up and freed here.
            unsafe {
                let mut pk: mbedtls_pk_context = core::mem::zeroed();
                mbedtls_pk_init(&mut pk);
                let verified = mbedtls_pk_parse_public_key(&mut pk, key.as_ptr(), key.len()) == 0
                    && mbedtls_pk_verify_ext(
                        crt.private_sig_pk,
                        crt.private_sig_opts,
                        &mut pk,
                        crt.private_sig_md,
                        hash.as_ptr(),
                        hash_len,
                        signature.as_ptr(),
                        signature.len(),
                    ) == 0;
                mbedtls_pk_free(&mut pk);
                verified
            }
        })
}

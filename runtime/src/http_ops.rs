//! HTTP client operations for the Coral runtime (L3.1).
//!
//! Provides `coral_http_get`, `coral_http_post`, and `coral_http_request` FFI
//! functions using the `ureq` crate for synchronous HTTP/1.1 requests.
//!
//! Success returns a map: `{"status": <int>, "body": <string>, "headers": <map>}`
//! Failure returns an error value: `err Http:Connection` or `err Http:Timeout` etc.

use crate::*;

/// Helper: extract a Rust `String` from a `ValueHandle` that should be a string.
fn string_from_handle(h: ValueHandle) -> Option<String> {
    if h.is_null() {
        return None;
    }
    let val = unsafe { &*h };
    if val.tag != ValueTag::String as u8 {
        return None;
    }
    Some(value_to_rust_string(val))
}

/// Helper: build a response map `{"status": N, "body": "...", "headers": {...}}`
fn build_response_map(status: u16, body: &str, header_pairs: &[(String, String)]) -> ValueHandle {
    // Build headers sub-map
    let header_entries: Vec<MapEntry> = header_pairs
        .iter()
        .map(|(k, v)| MapEntry {
            key: coral_make_string_from_rust(k),
            value: coral_make_string_from_rust(v),
        })
        .collect();
    let headers_map = coral_make_map(header_entries.as_ptr(), header_entries.len());

    let status_key = coral_make_string_from_rust("status");
    let status_val = coral_make_number(status as f64);
    let body_key = coral_make_string_from_rust("body");
    let body_val = coral_make_string_from_rust(body);
    let headers_key = coral_make_string_from_rust("headers");

    let entries = [
        MapEntry { key: status_key, value: status_val },
        MapEntry { key: body_key, value: body_val },
        MapEntry { key: headers_key, value: headers_map },
    ];
    coral_make_map(entries.as_ptr(), entries.len())
}

/// Helper: create an HTTP error value.
fn make_http_error(kind: &str) -> ValueHandle {
    let name = format!("Http:{}", kind);
    coral_make_error(0, name.as_ptr(), name.len())
}

/// Helper: execute a ureq request and return (status, body, headers) or error.
fn execute_request(request: ureq::Request) -> Result<(u16, String, Vec<(String, String)>), ValueHandle> {
    match request.call() {
        Ok(response) => {
            let status = response.status();
            let mut headers = Vec::new();
            for name in response.headers_names() {
                if let Some(val) = response.header(&name) {
                    headers.push((name, val.to_string()));
                }
            }
            let body = response.into_string().unwrap_or_default();
            Ok((status, body, headers))
        }
        Err(ureq::Error::Status(code, response)) => {
            // HTTP error status (4xx, 5xx) — still return a response map
            let mut headers = Vec::new();
            for name in response.headers_names() {
                if let Some(val) = response.header(&name) {
                    headers.push((name, val.to_string()));
                }
            }
            let body = response.into_string().unwrap_or_default();
            Ok((code, body, headers))
        }
        Err(ureq::Error::Transport(transport)) => {
            let kind = match transport.kind() {
                ureq::ErrorKind::Dns => "DnsError",
                ureq::ErrorKind::ConnectionFailed => "ConnectionFailed",
                ureq::ErrorKind::TooManyRedirects => "TooManyRedirects",
                ureq::ErrorKind::InvalidUrl => "InvalidUrl",
                ureq::ErrorKind::Io => "IoError",
                _ => "TransportError",
            };
            Err(make_http_error(kind))
        }
    }
}

// ========== FFI Functions ==========

/// `coral_http_get(url) -> map | error`
///
/// Performs an HTTP GET request to the given URL string.
/// Returns a map `{"status": N, "body": "...", "headers": {...}}` on success,
/// or an error value on failure.
#[unsafe(no_mangle)]
pub extern "C" fn coral_http_get(url_handle: ValueHandle) -> ValueHandle {
    let url = match string_from_handle(url_handle) {
        Some(u) => u,
        None => return make_http_error("InvalidUrl"),
    };
    let request = ureq::get(&url);
    match execute_request(request) {
        Ok((status, body, headers)) => build_response_map(status, &body, &headers),
        Err(error_val) => error_val,
    }
}

/// `coral_http_post(url, body) -> map | error`
///
/// Performs an HTTP POST request with the given body string.
#[unsafe(no_mangle)]
pub extern "C" fn coral_http_post(url_handle: ValueHandle, body_handle: ValueHandle) -> ValueHandle {
    let url = match string_from_handle(url_handle) {
        Some(u) => u,
        None => return make_http_error("InvalidUrl"),
    };
    let body = match string_from_handle(body_handle) {
        Some(b) => b,
        None => return make_http_error("InvalidBody"),
    };
    let request = ureq::post(&url).set("Content-Type", "application/octet-stream");
    match request.send_string(&body) {
        Ok(response) => {
            let status = response.status();
            let mut headers = Vec::new();
            for name in response.headers_names() {
                if let Some(val) = response.header(&name) {
                    headers.push((name, val.to_string()));
                }
            }
            let resp_body = response.into_string().unwrap_or_default();
            build_response_map(status, &resp_body, &headers)
        }
        Err(ureq::Error::Status(code, response)) => {
            let mut headers = Vec::new();
            for name in response.headers_names() {
                if let Some(val) = response.header(&name) {
                    headers.push((name, val.to_string()));
                }
            }
            let resp_body = response.into_string().unwrap_or_default();
            build_response_map(code, &resp_body, &headers)
        }
        Err(ureq::Error::Transport(transport)) => {
            let kind = match transport.kind() {
                ureq::ErrorKind::Dns => "DnsError",
                ureq::ErrorKind::ConnectionFailed => "ConnectionFailed",
                ureq::ErrorKind::InvalidUrl => "InvalidUrl",
                _ => "TransportError",
            };
            make_http_error(kind)
        }
    }
}

/// `coral_http_request(method, url, headers_map, body) -> map | error`
///
/// Generic HTTP request: method (string), url (string), headers (map or unit),
/// body (string or unit).
#[unsafe(no_mangle)]
pub extern "C" fn coral_http_request(
    method_handle: ValueHandle,
    url_handle: ValueHandle,
    headers_handle: ValueHandle,
    body_handle: ValueHandle,
) -> ValueHandle {
    let method = match string_from_handle(method_handle) {
        Some(m) => m,
        None => return make_http_error("InvalidMethod"),
    };
    let url = match string_from_handle(url_handle) {
        Some(u) => u,
        None => return make_http_error("InvalidUrl"),
    };

    let mut request = ureq::request(&method.to_uppercase(), &url);

    // Apply custom headers from a map value
    if !headers_handle.is_null() {
        let hval = unsafe { &*headers_handle };
        if hval.tag == ValueTag::Map as u8 {
            // Iterate map entries by calling coral_map_keys and reading
            let keys_handle = coral_map_keys(headers_handle);
            if !keys_handle.is_null() {
                let keys_val = unsafe { &*keys_handle };
                if keys_val.tag == ValueTag::List as u8 {
                    if let Some(list_obj) = list_from_value(keys_val) {
                        for key_h in &list_obj.items {
                            if let Some(key_str) = string_from_handle(*key_h) {
                                let val_h = coral_map_get(headers_handle, *key_h);
                                if let Some(val_str) = string_from_handle(val_h) {
                                    request = request.set(&key_str, &val_str);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Send with or without body
    let has_body = !body_handle.is_null() && {
        let bval = unsafe { &*body_handle };
        bval.tag == ValueTag::String as u8
    };

    let result = if has_body {
        let body = string_from_handle(body_handle).unwrap_or_default();
        request.send_string(&body)
    } else {
        request.call()
    };

    match result {
        Ok(response) => {
            let status = response.status();
            let mut resp_headers = Vec::new();
            for name in response.headers_names() {
                if let Some(val) = response.header(&name) {
                    resp_headers.push((name, val.to_string()));
                }
            }
            let resp_body = response.into_string().unwrap_or_default();
            build_response_map(status, &resp_body, &resp_headers)
        }
        Err(ureq::Error::Status(code, response)) => {
            let mut resp_headers = Vec::new();
            for name in response.headers_names() {
                if let Some(val) = response.header(&name) {
                    resp_headers.push((name, val.to_string()));
                }
            }
            let resp_body = response.into_string().unwrap_or_default();
            build_response_map(code, &resp_body, &resp_headers)
        }
        Err(ureq::Error::Transport(transport)) => {
            let kind = match transport.kind() {
                ureq::ErrorKind::Dns => "DnsError",
                ureq::ErrorKind::ConnectionFailed => "ConnectionFailed",
                ureq::ErrorKind::InvalidUrl => "InvalidUrl",
                _ => "TransportError",
            };
            make_http_error(kind)
        }
    }
}

// ========== Tests ==========

#[cfg(test)]
mod tests {
    use super::*;

    // FLAG_ERR = 0b0001_0000 = 0x10
    const FLAG_ERR_TEST: u8 = 0x10;

    fn is_error(h: ValueHandle) -> bool {
        if h.is_null() { return false; }
        let val = unsafe { &*h };
        val.tag == ValueTag::Unit as u8 && (val.flags & FLAG_ERR_TEST) != 0
    }

    #[test]
    fn l31_http_get_invalid_url_returns_error() {
        // Passing a non-string (number) should return an InvalidUrl error
        let num = coral_make_number(42.0);
        let result = coral_http_get(num);
        assert!(is_error(result), "expected error for non-string URL");
    }

    #[test]
    fn l31_http_get_bad_host_returns_error() {
        // A well-formed URL but unresolvable host
        let url = coral_make_string_from_rust("http://this-host-does-not-exist-12345.invalid/foo");
        let result = coral_http_get(url);
        assert!(is_error(result), "expected error for bad host");
    }

    #[test]
    fn l31_http_post_invalid_url_returns_error() {
        let url = coral_make_number(0.0);
        let body = coral_make_string_from_rust("hello");
        let result = coral_http_post(url, body);
        assert!(is_error(result), "expected error for non-string URL");
    }

    #[test]
    fn l31_http_request_invalid_method_returns_error() {
        let method = coral_make_number(0.0);
        let url = coral_make_string_from_rust("http://example.com");
        let result = coral_http_request(method, url, std::ptr::null_mut(), std::ptr::null_mut());
        assert!(is_error(result), "expected error for invalid method");
    }

    #[test]
    fn l31_http_get_null_returns_error() {
        let result = coral_http_get(std::ptr::null_mut());
        assert!(is_error(result), "expected error for null URL");
    }
}

//! Just enough HTTP/1.1 for the bridge's page: one request a connection,
//! read whole, answered whole, then closed. The only client is the page
//! the bridge itself serves, which sends a `Content-Length` with every body.

use serde_json::Value;
use tokio::{io::AsyncReadExt, net::TcpStream};

/// The most a request's head may be.
const HEAD_LIMIT: usize = 16 * 1024;
/// The most a request's body may be. A wallet's own answers are small (a
/// signature, a list of accounts); a node's reply the wallet passes on can
/// be larger.
const BODY_LIMIT: usize = 4 * 1024 * 1024;

/// A request that is not one this server reads.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Malformed;

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Request {
    pub(super) method: String,
    /// The path, without its query.
    pub(super) path: String,
    /// The query, without its `?`; empty when there is none.
    pub(super) query: String,
    pub(super) host: Option<String>,
    pub(super) content_type: Option<String>,
    pub(super) body: Vec<u8>,
}

impl Request {
    /// The value of the query's member `name`.
    pub(super) fn query_value(&self, name: &str) -> Option<&str> {
        self.query
            .split('&')
            .filter_map(|member| member.split_once('='))
            .find_map(|(key, value)| (key == name).then_some(value))
    }

    /// Whether the body is declared as JSON.
    pub(super) fn is_json(&self) -> bool {
        self.content_type
            .as_deref()
            .and_then(|kind| kind.split(';').next())
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
    }
}

/// A request's head: the request without its body, how many bytes the head
/// took, and how many the body is to be.
#[derive(Debug, PartialEq, Eq)]
struct Head {
    request: Request,
    length: usize,
    body_length: usize,
}

/// Reads the head at the start of `bytes`; `None` while it is not all there.
fn head(bytes: &[u8]) -> Result<Option<Head>, Malformed> {
    const END: &[u8] = b"\r\n\r\n";
    let Some(end) = bytes.windows(END.len()).position(|window| window == END) else {
        return if bytes.len() > HEAD_LIMIT {
            Err(Malformed)
        } else {
            Ok(None)
        };
    };
    let text = std::str::from_utf8(&bytes[..end]).map_err(|_| Malformed)?;
    let mut lines = text.split("\r\n");
    let mut line = lines.next().ok_or(Malformed)?.split(' ');
    let (Some(method), Some(target), Some(version), None) =
        (line.next(), line.next(), line.next(), line.next())
    else {
        return Err(Malformed);
    };
    if !version.starts_with("HTTP/1.") || !target.starts_with('/') {
        return Err(Malformed);
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut request = Request {
        method: method.to_owned(),
        path: path.to_owned(),
        query: query.to_owned(),
        ..Request::default()
    };
    let mut body_length = 0;
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(Malformed)?;
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "host" => request.host = Some(value.to_owned()),
            "content-type" => request.content_type = Some(value.to_owned()),
            "content-length" => body_length = value.parse().map_err(|_| Malformed)?,
            // A body in chunks is not one the page sends.
            "transfer-encoding" => return Err(Malformed),
            _ => {}
        }
    }
    if body_length > BODY_LIMIT {
        return Err(Malformed);
    }
    Ok(Some(Head {
        request,
        length: end + END.len(),
        body_length,
    }))
}

/// Reads one whole request from `stream`.
pub(super) async fn read(stream: &mut TcpStream) -> Result<Request, Malformed> {
    let mut bytes = Vec::with_capacity(2048);
    let mut chunk = [0_u8; 4096];
    let Head {
        mut request,
        length,
        body_length,
    } = loop {
        if let Some(head) = head(&bytes)? {
            break head;
        }
        match stream.read(&mut chunk).await {
            Ok(read) if read > 0 => bytes.extend_from_slice(&chunk[..read]),
            _ => return Err(Malformed),
        }
    };
    let mut body = bytes.split_off(length);
    while body.len() < body_length {
        match stream.read(&mut chunk).await {
            Ok(read) if read > 0 => body.extend_from_slice(&chunk[..read]),
            _ => return Err(Malformed),
        }
    }
    body.truncate(body_length);
    request.body = body;
    Ok(request)
}

/// An answer to a request.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Response {
    status: u16,
    content_type: Option<&'static str>,
    body: Vec<u8>,
}

impl Response {
    /// `status` and nothing else.
    pub(super) fn empty(status: u16) -> Self {
        Self {
            status,
            content_type: None,
            body: Vec::new(),
        }
    }

    pub(super) fn json(body: &Value) -> Self {
        Self {
            status: 200,
            content_type: Some("application/json"),
            body: body.to_string().into_bytes(),
        }
    }

    pub(super) fn html(body: &str) -> Self {
        Self {
            status: 200,
            content_type: Some("text/html; charset=utf-8"),
            body: body.as_bytes().to_vec(),
        }
    }

    /// The answer as it goes down the connection. Nothing of it may be
    /// kept by the browser or shown inside another site's page.
    pub(super) fn encode(&self) -> Vec<u8> {
        let reason = match self.status {
            200 => "OK",
            204 => "No Content",
            400 => "Bad Request",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            409 => "Conflict",
            _ => "Error",
        };
        let mut head = format!(
            "HTTP/1.1 {} {reason}\r\n\
             Connection: close\r\n\
             Cache-Control: no-store\r\n\
             Referrer-Policy: no-referrer\r\n\
             X-Content-Type-Options: nosniff\r\n\
             X-Frame-Options: DENY\r\n\
             Content-Security-Policy: frame-ancestors 'none'\r\n",
            self.status
        );
        if let Some(content_type) = self.content_type {
            head.push_str(&format!("Content-Type: {content_type}\r\n"));
        }
        // A 204 has no body and says nothing of one.
        if self.status != 204 {
            head.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
        }
        head.push_str("\r\n");
        let mut bytes = head.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_head_is_read_once_it_is_whole() {
        let whole = b"POST /abc/reply?page=p1&x=2 HTTP/1.1\r\nHost: 127.0.0.1:47821\r\n\
                      Content-Type: application/json; charset=utf-8\r\ncontent-length: 7\r\n\r\n{\"a\":1}";
        // Not all there yet.
        assert_eq!(head(&whole[..40]), Ok(None));
        let read = head(whole).unwrap().unwrap();
        assert_eq!(read.request.method, "POST");
        assert_eq!(read.request.path, "/abc/reply");
        assert_eq!(read.request.query_value("page"), Some("p1"));
        assert_eq!(read.request.query_value("x"), Some("2"));
        assert_eq!(read.request.query_value("y"), None);
        assert_eq!(read.request.host.as_deref(), Some("127.0.0.1:47821"));
        assert!(read.request.is_json());
        assert_eq!(read.body_length, 7);
        assert_eq!(&whole[read.length..], b"{\"a\":1}");

        let plain = head(b"GET /abc HTTP/1.1\r\nHost: localhost:1\r\n\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(plain.request.query, "");
        assert_eq!(plain.body_length, 0);
        assert!(!plain.request.is_json());
    }

    #[test]
    fn what_is_not_a_request_of_the_page_is_refused() {
        for bytes in [
            &b"GET\r\n\r\n"[..],
            b"GET abc HTTP/1.1\r\n\r\n",
            b"GET / SPDY/3\r\n\r\n",
            b"GET / HTTP/1.1\r\nno colon\r\n\r\n",
            b"POST / HTTP/1.1\r\nContent-Length: many\r\n\r\n",
            b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
            b"POST / HTTP/1.1\r\nContent-Length: 99999999\r\n\r\n",
        ] {
            assert_eq!(
                head(bytes),
                Err(Malformed),
                "{}",
                String::from_utf8_lossy(bytes)
            );
        }
        // A head that never ends is given up on.
        assert_eq!(head(&vec![b'a'; HEAD_LIMIT + 1]), Err(Malformed));
    }

    #[test]
    fn answers_say_how_long_they_are_and_are_not_kept() {
        let page = String::from_utf8(Response::html("<p>hi</p>").encode()).unwrap();
        assert!(page.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(page.contains("Content-Type: text/html; charset=utf-8\r\n"));
        assert!(page.contains("Content-Length: 9\r\n"));
        assert!(page.contains("Cache-Control: no-store\r\n"));
        assert!(page.contains("X-Frame-Options: DENY\r\n"));
        assert!(page.ends_with("\r\n\r\n<p>hi</p>"));

        let nothing = String::from_utf8(Response::empty(204).encode()).unwrap();
        assert!(nothing.starts_with("HTTP/1.1 204 No Content\r\n"));
        assert!(!nothing.contains("Content-Length"));
        assert!(nothing.ends_with("\r\n\r\n"));

        let json = String::from_utf8(Response::json(&serde_json::json!([1])).encode()).unwrap();
        assert!(json.ends_with("Content-Length: 3\r\n\r\n[1]"));
    }
}

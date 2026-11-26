//! Requests, responses, forms and cookies.
//!
//! The application handles a complete request in memory: the server reads the body
//! with a size limit first. Thus the handlers are simple synchronous functions, and
//! the tests can call them without a network connection.

use std::collections::HashMap;

/// An HTTP request with the complete body.
#[derive(Debug, Clone, Default)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// The query string without `?`.
    pub query: String,
    /// The header names are in lower case.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    /// True if the client connects from the same computer.
    pub peer_is_loopback: bool,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn cookies(&self) -> HashMap<String, String> {
        let mut cookies = HashMap::new();
        for header in self.headers.iter().filter(|(n, _)| n.eq_ignore_ascii_case("cookie")) {
            for part in header.1.split(';') {
                if let Some((name, value)) = part.trim().split_once('=') {
                    let value = value.trim().trim_matches('"');
                    cookies.entry(name.trim().to_string()).or_insert_with(|| value.to_string());
                }
            }
        }
        cookies
    }

    pub fn cookie(&self, name: &str) -> Option<String> {
        self.cookies().remove(name)
    }
}

/// An HTTP response.
#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The attributes of a cookie.
pub struct Cookie<'a> {
    pub name: &'a str,
    pub value: &'a str,
    pub max_age: i64,
    pub path: &'a str,
    pub secure: bool,
}

impl Response {
    pub fn new(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Response {
        Response { status, headers: vec![("content-type".into(), content_type.into())], body: body.into() }
    }

    pub fn text(status: u16, body: &str) -> Response {
        Response::new(status, "text/plain; charset=utf-8", body)
    }

    pub fn html(status: u16, body: String) -> Response {
        Response::new(status, "text/html; charset=utf-8", body)
    }

    pub fn json(status: u16, value: &serde_json::Value) -> Response {
        Response::new(status, "application/json", value.to_string())
    }

    /// A redirect with "303 See Other".
    pub fn redirect(location: &str) -> Response {
        Response { status: 303, headers: vec![("location".into(), location.into())], body: Vec::new() }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    pub fn set_header(&mut self, name: &str, value: &str) {
        self.headers.push((name.to_lowercase(), value.to_string()));
    }

    /// Add a cookie with the flags `HttpOnly` and `SameSite=Strict`.
    pub fn set_cookie(&mut self, cookie: &Cookie<'_>) {
        let mut value = format!(
            "{}={}; HttpOnly; Max-Age={}; Path={}; SameSite=Strict",
            cookie.name, cookie.value, cookie.max_age, cookie.path
        );
        if cookie.secure {
            value.push_str("; Secure");
        }
        self.set_header("set-cookie", &value);
    }

    /// Tell the browser to delete a cookie.
    pub fn delete_cookie(&mut self, name: &str, path: &str, secure: bool) {
        let mut value = format!(
            "{name}=\"\"; expires=Thu, 01 Jan 1970 00:00:00 GMT; HttpOnly; Max-Age=0; Path={path}; SameSite=Strict"
        );
        if secure {
            value.push_str("; Secure");
        }
        self.set_header("set-cookie", &value);
    }

    pub fn text_body(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// An uploaded file.
#[derive(Debug, Clone)]
pub struct Upload {
    pub filename: Option<String>,
    pub data: Vec<u8>,
}

/// The values of a query string or of a form.
#[derive(Debug, Clone, Default)]
pub struct Params {
    pub pairs: Vec<(String, String)>,
    pub files: Vec<(String, Upload)>,
}

impl Params {
    pub fn from_query(query: &str) -> Params {
        Params { pairs: form_urlencoded::parse(query.as_bytes()).into_owned().collect(), files: Vec::new() }
    }

    /// The last value of a parameter.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.pairs.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    pub fn all(&self, name: &str) -> Vec<&str> {
        self.pairs.iter().filter(|(n, _)| n == name).map(|(_, v)| v.as_str()).collect()
    }

    pub fn file(&self, name: &str) -> Option<&Upload> {
        self.files.iter().rev().find(|(n, _)| n == name).map(|(_, f)| f)
    }

    pub fn has(&self, name: &str) -> bool {
        self.pairs.iter().any(|(n, _)| n == name) || self.files.iter().any(|(n, _)| n == name)
    }
}

/// Parse the body of a form: `application/x-www-form-urlencoded` or
/// `multipart/form-data`. Return `None` if the body is not valid.
pub fn parse_form(content_type: Option<&str>, body: &[u8]) -> Option<Params> {
    let content_type = content_type.unwrap_or("");
    let media = content_type.split(';').next().unwrap_or("").trim().to_lowercase();
    match media.as_str() {
        "multipart/form-data" => {
            let boundary = content_type.split(';').skip(1).find_map(|p| {
                let (name, value) = p.trim().split_once('=')?;
                name.trim()
                    .eq_ignore_ascii_case("boundary")
                    .then(|| value.trim().trim_matches('"').to_string())
            })?;
            parse_multipart(body, &boundary)
        }
        _ => Some(Params { pairs: form_urlencoded::parse(body).into_owned().collect(), files: Vec::new() }),
    }
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (from..=haystack.len() - needle.len()).find(|&i| &haystack[i..i + needle.len()] == needle)
}

/// Read a parameter of a `Content-Disposition` header, for example `name="x"`.
fn disposition_param(header: &str, param: &str) -> Option<String> {
    let mut rest = header;
    while let Some(pos) = rest.find(';') {
        rest = &rest[pos + 1..];
        let trimmed = rest.trim_start();
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case(param) {
            continue;
        }
        let value = value.trim_start();
        if let Some(quoted) = value.strip_prefix('"') {
            let mut out = String::new();
            let mut chars = quoted.chars();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        if let Some(next) = chars.next() {
                            out.push(next);
                        }
                    }
                    '"' => break,
                    _ => out.push(c),
                }
            }
            return Some(out);
        }
        return Some(value.split(';').next().unwrap_or("").trim().to_string());
    }
    None
}

fn parse_multipart(body: &[u8], boundary: &str) -> Option<Params> {
    if boundary.is_empty() || boundary.len() > 200 {
        return None;
    }
    let delimiter = format!("--{boundary}").into_bytes();
    let mut params = Params::default();
    let mut pos = find(body, &delimiter, 0)? + delimiter.len();
    loop {
        if body.get(pos..pos + 2) == Some(b"--") {
            return Some(params);
        }
        // Skip the line end after the delimiter.
        if body.get(pos..pos + 2) == Some(b"\r\n") {
            pos += 2;
        } else {
            return None;
        }
        let header_end = find(body, b"\r\n\r\n", pos)?;
        let headers = String::from_utf8_lossy(&body[pos..header_end]).into_owned();
        let content_start = header_end + 4;
        let mut next_delimiter = b"\r\n".to_vec();
        next_delimiter.extend_from_slice(&delimiter);
        let content_end = find(body, &next_delimiter, content_start)?;
        let content = &body[content_start..content_end];
        pos = content_end + next_delimiter.len();

        let disposition = headers
            .split("\r\n")
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim().eq_ignore_ascii_case("content-disposition").then(|| value.trim().to_string())
            })
            .unwrap_or_default();
        let Some(name) = disposition_param(&disposition, "name") else {
            continue;
        };
        match disposition_param(&disposition, "filename") {
            Some(filename) => params.files.push((
                name,
                Upload { filename: Some(filename).filter(|f| !f.is_empty()), data: content.to_vec() },
            )),
            None => params.pairs.push((name, String::from_utf8_lossy(content).into_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipart() {
        let body = b"--XyZ\r\nContent-Disposition: form-data; name=\"csrf_token\"\r\n\r\nabc\r\n\
--XyZ\r\nContent-Disposition: form-data; name=\"statement\"; filename=\"a.xml\"\r\n\
Content-Type: application/xml\r\n\r\n<x>\r\n</x>\r\n--XyZ--\r\n";
        let params = parse_form(Some("multipart/form-data; boundary=XyZ"), body).unwrap();
        assert_eq!(params.get("csrf_token"), Some("abc"));
        let file = params.file("statement").unwrap();
        assert_eq!(file.filename.as_deref(), Some("a.xml"));
        assert_eq!(file.data, b"<x>\r\n</x>");
    }

    #[test]
    fn urlencoded() {
        let params = parse_form(Some("application/x-www-form-urlencoded"), b"a=1&b=x+y&a=2").unwrap();
        assert_eq!(params.get("a"), Some("2"));
        assert_eq!(params.all("a"), vec!["1", "2"]);
        assert_eq!(params.get("b"), Some("x y"));
    }

    #[test]
    fn cookies() {
        let request = Request {
            headers: vec![("cookie".into(), "a=1; feljosit_session=tok; b=\"q\"".into())],
            ..Request::default()
        };
        assert_eq!(request.cookie("feljosit_session").as_deref(), Some("tok"));
        assert_eq!(request.cookie("b").as_deref(), Some("q"));
    }
}

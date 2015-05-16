use std::borrow::Cow;
use std::sync::RwLock;
use std::collections::HashMap;
use std::collections::hash_map::Entry::{Occupied, Vacant};
use std::path::Path;
use serialize::Encodable;
use hyper::status::StatusCode;
use hyper::server::Response as HyperResponse;
use hyper::header::{
    Headers, Date, HttpDate, Server, ContentType, ContentLength, Header, HeaderFormat
};
use hyper::net::{Fresh, Streaming};
use time;
use mimes::MediaType;
use mustache;
use mustache::Template;
use std::io::{self, Read, Write, copy};
use std::fs::File;
use std::any::Any;
use {NickelError, Halt, MiddlewareResult, Responder};
use modifier::Modifier;

pub type TemplateCache = RwLock<HashMap<String, Template>>;

///A container for the response
pub struct Response<'a, T: 'static + Any = Fresh> {
    ///the original `hyper::server::Response`
    origin: HyperResponse<'a, T>,
    templates: &'a TemplateCache
}

impl<'a> Response<'a, Fresh> {
    pub fn from_internal<'c, 'd>(response: HyperResponse<'c, Fresh>,
                                 templates: &'c TemplateCache)
                                -> Response<'c, Fresh> {
        Response {
            origin: response,
            templates: templates
        }
    }

    /// Get a mutable reference to the status.
    pub fn status_mut(&mut self) -> &mut StatusCode {
        self.origin.status_mut()
    }

    /// Get a mutable reference to the Headers.
    pub fn headers_mut(&mut self) -> &mut Headers {
        self.origin.headers_mut()
    }

    /// Modify the response with the provided data.
    ///
    /// # Examples
    /// ```{rust}
    /// extern crate hyper;
    /// #[macro_use] extern crate nickel;
    ///
    /// use nickel::{Nickel, HttpRouter, MediaType};
    /// use nickel::status::StatusCode;
    /// use hyper::header::Location;
    ///
    /// fn main() {
    ///     let mut server = Nickel::new();
    ///     server.get("/a", middleware! { |_, mut res|
    ///             // set the Status
    ///         res.set(StatusCode::PermanentRedirect)
    ///             // update a Header value
    ///            .set(Location("http://nickel.rs".into()));
    ///
    ///         ""
    ///     });
    ///
    ///     server.get("/b", middleware! { |_, mut res|
    ///             // setting the content type
    ///         res.set(MediaType::Json);
    ///
    ///         "{'foo': 'bar'}"
    ///     });
    ///
    ///     // ...
    /// }
    /// ```
    pub fn set<T: Modifier<Response<'a>>>(&mut self, attribute: T) -> &mut Response<'a> {
        attribute.modify(self);
        self
    }

    /// Writes a response
    ///
    /// # Examples
    /// ```{rust}
    /// use nickel::{Request, Response, MiddlewareResult, Halt};
    ///
    /// fn handler<'a>(_: &mut Request, res: Response<'a>) -> MiddlewareResult<'a> {
    ///     res.send("hello world")
    /// }
    /// ```
    #[inline]
    pub fn send<T: Responder>(self, data: T) -> MiddlewareResult<'a> {
        data.respond(self)
    }

    /// Writes a file to the output.
    ///
    /// # Examples
    /// ```{rust}
    /// use nickel::{Request, Response, MiddlewareResult, Halt};
    /// use nickel::status::StatusCode;
    /// use std::path::Path;
    ///
    /// fn handler<'a>(_: &mut Request, mut res: Response<'a>) -> MiddlewareResult<'a> {
    ///     let favicon = Path::new("/assets/favicon.ico");
    ///     res.send_file(favicon)
    /// }
    /// ```
    pub fn send_file(mut self, path: &Path) -> MiddlewareResult<'a> {
        // Chunk the response
        self.origin.headers_mut().remove::<ContentLength>();
        // Determine content type by file extension or default to binary
        let mime = mime_from_filename(path).unwrap_or(MediaType::Bin);
        self.set(mime);

        let mut file = try_with!(self, {
            File::open(path).map_err(|e| format!("Failed to send file '{:?}': {}",
                                                 path, e))
        });

        let mut stream = try!(self.start());
        match copy(&mut file, &mut stream) {
            Ok(_) => Ok(Halt(stream)),
            Err(e) => stream.bail(format!("Failed to send file: {}", e))
        }
    }

    // TODO: This needs to be more sophisticated to return the correct headers
    // not just "some headers" :)
    //
    // Also, it should only set them if not already set.
    fn set_fallback_headers(&mut self) {
        self.set_header_fallback(|| Date(HttpDate(time::now_utc())));
        self.set_header_fallback(|| Server("Nickel".to_string()));
        self.set_header_fallback(|| ContentType(MediaType::Html.into()));
    }

    /// Return an error with the appropriate status code for error handlers to
    /// provide output for.
    pub fn error<T>(self, status: StatusCode, message: T) -> MiddlewareResult<'a>
            where T: Into<Cow<'static, str>> {
        Err(NickelError::new(self, message, status))
    }

    /// Sets the header if not already set.
    ///
    /// If the header is not set then `f` will be called.
    /// Renders the given template bound with the given data.
    ///
    /// # Examples
    /// ```{rust}
    /// #[macro_use] extern crate nickel;
    /// extern crate hyper;
    ///
    /// use nickel::{Nickel, HttpRouter, MediaType};
    /// use hyper::header::ContentType;
    ///
    /// fn main() {
    ///     let mut server = Nickel::new();
    ///     server.get("/", middleware! { |_, mut res|
    ///         res.set(MediaType::Html);
    ///         res.set_header_fallback(|| {
    ///             panic!("Should not get called");
    ///             ContentType(MediaType::Txt.into())
    ///         });
    ///
    ///         "<h1>Hello World</h1>"
    ///     });
    ///
    ///     // ...
    /// }
    /// ```
    pub fn set_header_fallback<F, H>(&mut self, f: F)
            where H: Header + HeaderFormat, F: FnOnce() -> H {
        let headers = self.origin.headers_mut();
        if !headers.has::<H>() { headers.set(f()) }
    }

    /// Renders the given template bound with the given data.
    ///
    /// # Examples
    /// ```{rust}
    /// use std::collections::HashMap;
    /// use nickel::{Request, Response, MiddlewareResult, Halt};
    ///
    /// fn handler<'a>(_: &mut Request, mut res: Response<'a>) -> MiddlewareResult<'a> {
    ///     let mut data = HashMap::new();
    ///     data.insert("name", "user");
    ///     res.render("examples/assets/template.tpl", &data)
    /// }
    /// ```
    pub fn render<T, P>(self, path: P, data: &T) -> MiddlewareResult<'a>
            where T: Encodable, P: AsRef<str> + Into<String> {
        fn render<'a, T>(res: Response<'a>, template: &Template, data: &T)
                -> MiddlewareResult<'a> where T: Encodable {
            let mut stream = try!(res.start());
            match template.render(&mut stream, data) {
                Ok(()) => Ok(Halt(stream)),
                Err(e) => stream.bail(format!("Problem rendering template: {:?}", e))
            }
        }

        // Fast path doesn't need writer lock
        if let Some(t) = self.templates.read().unwrap().get(path.as_ref()) {
            return render(self, t, data);
        }

        // We didn't find the template, get writers lock
        let mut templates = self.templates.write().unwrap();

        // Additional clone required for now as the entry api doesn't give us a key ref
        let path = path.into();

        // Search again incase there was a race to compile the template
        let template = match templates.entry(path.clone()) {
            Vacant(entry) => {
                let template = try_with!(self, {
                    mustache::compile_path(&path)
                             .map_err(|e| format!("Failed to compile template '{}': {:?}",
                                            path, e))
                });
                entry.insert(template)
            },
            Occupied(entry) => entry.into_mut()
        };

        render(self, template, data)
    }

    pub fn start(mut self) -> Result<Response<'a, Streaming>, NickelError<'a>> {
        self.set_fallback_headers();

        let Response { origin, templates } = self;
        match origin.start() {
            Ok(origin) => Ok(Response { origin: origin, templates: templates }),
            Err(e) =>
                unsafe {
                    Err(NickelError::without_response(format!("Failed to start response: {}", e)))
                }
        }
    }
}

impl<'a, 'b> Write for Response<'a, Streaming> {
    #[inline(always)]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.origin.write(buf)
    }

    #[inline(always)]
    fn flush(&mut self) -> io::Result<()> {
        self.origin.flush()
    }
}

impl<'a, 'b> Response<'a, Streaming> {
    /// In the case of an unrecoverable error while a stream is already in
    /// progress, there is no standard way to signal to the client that an
    /// error has occurred. `bail` will drop the connection and log an error
    /// message.
    pub fn bail<T>(self, message: T) -> MiddlewareResult<'a>
            where T: Into<Cow<'static, str>> {
        let _ = self.end();
        unsafe { Err(NickelError::without_response(message)) }
    }

    /// Flushes all writing of a response to the client.
    pub fn end(self) -> io::Result<()> {
        self.origin.end()
    }
}

impl <'a, T: 'static + Any> Response<'a, T> {
    /// The status of this response.
    pub fn status(&self) -> StatusCode {
        self.origin.status()
    }

    /// The headers of this response.
    pub fn headers(&self) -> &Headers {
        self.origin.headers()
    }
}

fn mime_from_filename<P: AsRef<Path>>(path: P) -> Option<MediaType> {
    path.as_ref()
        .extension()
        .and_then(|os| os.to_str())
        // Lookup mime from file extension
        .and_then(|s| s.parse().ok())
}

#[test]
fn matches_content_type () {
    assert_eq!(Some(MediaType::Txt), mime_from_filename("test.txt"));
    assert_eq!(Some(MediaType::Json), mime_from_filename("test.json"));
    assert_eq!(Some(MediaType::Bin), mime_from_filename("test.bin"));
}

mod modifier_impls {
    use hyper::header::*;
    use hyper::status::StatusCode;
    use modifier::Modifier;
    use {Response, MediaType};

    impl<'a> Modifier<Response<'a>> for StatusCode {
        fn modify(self, res: &mut Response<'a>) {
            *res.status_mut() = self
        }
    }

    impl<'a> Modifier<Response<'a>> for MediaType {
        fn modify(self, res: &mut Response<'a>) {
            ContentType(self.into()).modify(res)
        }
    }

    macro_rules! header_modifiers {
        ($($t:ty),+) => (
            $(
                impl<'a> Modifier<Response<'a>> for $t {
                    fn modify(self, res: &mut Response<'a>) {
                        res.headers_mut().set(self)
                    }
                }
            )+
        )
    }

    header_modifiers! {
        Accept,
        AccessControlAllowHeaders,
        AccessControlAllowMethods,
        AccessControlAllowOrigin,
        AccessControlMaxAge,
        AccessControlRequestHeaders,
        AccessControlRequestMethod,
        AcceptCharset,
        AcceptEncoding,
        AcceptLanguage,
        AcceptRanges,
        Allow,
        Authorization<Basic>,
        Authorization<String>,
        CacheControl,
        Cookie,
        Connection,
        ContentEncoding,
        ContentLanguage,
        ContentLength,
        ContentType,
        Date,
        ETag,
        Expect,
        Expires,
        From,
        Host,
        IfMatch,
        IfModifiedSince,
        IfNoneMatch,
        IfRange,
        IfUnmodifiedSince,
        LastModified,
        Location,
        Pragma,
        Referer,
        Server,
        SetCookie,
        TransferEncoding,
        Upgrade,
        UserAgent,
        Vary
    }
}

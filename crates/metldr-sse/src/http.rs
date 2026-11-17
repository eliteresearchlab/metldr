//! The one HTTP request the provider clients make: a bounded JSON POST that must answer with an
//! event stream, cancelled by the caller's stop flag and by a whole-request timeout. Native
//! builds use reqwest with rustls and no proxy or redirects; the extension's wasm uses `fetch`
//! with an AbortController, so neither needs a runtime timer of its own.

use std::{
    future::Future,
    pin::pin,
    sync::atomic::{AtomicBool, Ordering},
    task::Poll,
};

pub type Result<T> = std::result::Result<T, String>;

const TRUST_BOUND: usize = 16_384;

/// One provider endpoint with its fixed headers; cheap to clone on native builds.
#[derive(Clone)]
pub struct Http {
    provider: &'static str,
    url: String,
    inner: platform::Client,
}

pub struct Options<'a> {
    /// Names the provider in every error, as in "Chat transport failed".
    pub provider: &'static str,
    /// An endpoint the caller already validated.
    pub url: &'a str,
    pub timeout_secs: u16,
    /// Port of a plain `http://localhost` endpoint, pinned to 127.0.0.1 so the name cannot
    /// resolve off the machine.
    pub localhost_port: Option<u16>,
    pub trust_ca_pem: Option<&'a [u8]>,
    /// Sent with every request, besides content type and accept.
    pub headers: &'a [(&'static str, &'a str)],
    /// The credential header, never shown in debug output.
    pub secret: Option<(&'static str, &'a str)>,
}

impl Http {
    pub fn new(options: Options<'_>) -> Result<Self> {
        let provider = options.provider;
        if let Some((_, value)) = options.secret
            && (value.is_empty()
                || value.len() > 4096
                || !value.bytes().all(|b| b == b' ' || b.is_ascii_graphic()))
        {
            return Err(format!("invalid {provider} credential"));
        }
        if options
            .trust_ca_pem
            .is_some_and(|pem| pem.len() > TRUST_BOUND)
        {
            return Err(format!("{provider} trust certificate exceeds bound"));
        }
        Ok(Self {
            provider,
            url: options.url.into(),
            inner: platform::Client::new(&options)?,
        })
    }

    /// Posts `body` and returns the response stream once the endpoint answered 200 with
    /// `text/event-stream`.
    pub async fn events(&self, body: Vec<u8>, stop: &AtomicBool) -> Result<Events> {
        let provider = self.provider;
        let sending = self.inner.post(&self.url, body);
        let response = race(stop, sending)
            .await
            .ok_or_else(|| format!("{provider} request cancelled"))?
            .map_err(|_| format!("{provider} transport failed"))?;
        if response.status() != 200 {
            return Err(format!("{provider} endpoint returned an error"));
        }
        if response
            .content_type()
            .as_deref()
            .and_then(|v| v.split(';').next())
            .is_none_or(|v| !v.trim().eq_ignore_ascii_case("text/event-stream"))
        {
            return Err(format!(
                "{provider} endpoint did not return an event stream"
            ));
        }
        Ok(Events { provider, response })
    }
}

pub struct Events {
    provider: &'static str,
    response: platform::Response,
}

impl Events {
    /// The next body bytes, or `None` at the end of the stream.
    pub async fn chunk(&mut self, stop: &AtomicBool) -> Result<Option<impl AsRef<[u8]> + use<>>> {
        let provider = self.provider;
        race(stop, self.response.chunk())
            .await
            .ok_or_else(|| format!("{provider} request cancelled"))?
            .map_err(|_| format!("{provider} stream transport failed"))
    }
}

impl Events {
    /// Feeds each event of the stream to `each` until the stream ends, holding the body to
    /// `bound` bytes and checking `stop` between events.
    pub async fn frames<E: From<String>>(
        mut self,
        stop: &AtomicBool,
        mut sse: crate::Sse,
        bound: usize,
        mut each: impl FnMut(&str, &str) -> std::result::Result<(), E>,
    ) -> std::result::Result<(), E> {
        let provider = self.provider;
        let mut bytes = 0usize;
        while let Some(chunk) = self.chunk(stop).await? {
            let mut rest = chunk.as_ref();
            bytes = bytes.saturating_add(rest.len());
            if bytes > bound {
                return Err(format!("{provider} stream exceeds bound").into());
            }
            while let Some((event, data)) = sse.next(&mut rest)? {
                stopped(provider, stop)?;
                each(&event, &data)?;
            }
        }
        stopped(provider, stop)?;
        Ok(sse.finish()?)
    }
}

fn stopped(provider: &str, stop: &AtomicBool) -> Result<()> {
    match stop.load(Ordering::Acquire) {
        true => Err(format!("{provider} request cancelled")),
        false => Ok(()),
    }
}

/// Runs `work` until it finishes or `stop` is set, checking the flag first and then every 20 ms.
async fn race<T>(stop: &AtomicBool, work: impl Future<Output = T>) -> Option<T> {
    let mut work = pin!(work);
    let mut stopped = pin!(async {
        while !stop.load(Ordering::Acquire) {
            platform::sleep_ms(20).await;
        }
    });
    std::future::poll_fn(|cx| {
        if stopped.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        work.as_mut().poll(cx).map(Some)
    })
    .await
}

#[cfg(not(target_arch = "wasm32"))]
mod platform {
    use super::{Options, Result};

    #[derive(Clone)]
    pub struct Client {
        client: reqwest::Client,
        headers: reqwest::header::HeaderMap,
    }

    pub struct Response(reqwest::Response);

    impl Client {
        pub fn new(options: &Options<'_>) -> Result<Self> {
            let provider = options.provider;
            let mut headers = reqwest::header::HeaderMap::new();
            let fixed = options.headers.iter().copied().chain([
                ("content-type", "application/json"),
                ("accept", "text/event-stream"),
            ]);
            for (name, value) in fixed {
                let value = reqwest::header::HeaderValue::from_str(value)
                    .map_err(|_| format!("invalid {provider} header"))?;
                headers.insert(name, value);
            }
            if let Some((name, value)) = options.secret {
                let mut value = reqwest::header::HeaderValue::from_str(value)
                    .map_err(|_| format!("invalid {provider} credential"))?;
                value.set_sensitive(true);
                headers.insert(name, value);
            }
            let mut client = reqwest::Client::builder()
                .tls_backend_rustls()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(std::time::Duration::from_secs(options.timeout_secs.into()));
            if let Some(port) = options.localhost_port {
                client = client.resolve(
                    "localhost",
                    std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                );
            }
            if let Some(pem) = options.trust_ca_pem {
                let certificates = reqwest::Certificate::from_pem_bundle(pem)
                    .map_err(|_| format!("invalid {provider} trust certificate"))?;
                if certificates.is_empty() {
                    return Err(format!("empty {provider} trust certificate"));
                }
                client = client.tls_certs_only(certificates);
            }
            let client = client
                .build()
                .map_err(|_| format!("{provider} client unavailable"))?;
            Ok(Self { client, headers })
        }

        pub async fn post(&self, url: &str, body: Vec<u8>) -> reqwest::Result<Response> {
            let request = self
                .client
                .post(url)
                .headers(self.headers.clone())
                .body(body);
            request.send().await.map(Response)
        }
    }

    impl Response {
        pub fn status(&self) -> u16 {
            self.0.status().as_u16()
        }

        pub fn content_type(&self) -> Option<String> {
            let value = self.0.headers().get("content-type")?;
            value.to_str().ok().map(str::to_owned)
        }

        pub async fn chunk(&mut self) -> reqwest::Result<Option<impl AsRef<[u8]> + use<>>> {
            self.0.chunk().await
        }
    }

    pub async fn sleep_ms(ms: u64) {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
    }
}

#[cfg(target_arch = "wasm32")]
mod platform {
    use super::{Options, Result};
    use wasm_bindgen::{JsCast, JsValue, prelude::wasm_bindgen};
    use wasm_bindgen_futures::JsFuture;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_name = fetch)]
        fn fetch(input: &str, init: &web_sys::RequestInit) -> js_sys::Promise;
        #[wasm_bindgen(js_name = setTimeout)]
        fn set_timeout(handler: &js_sys::Function, ms: i32) -> JsValue;
    }

    #[derive(Clone)]
    pub struct Client {
        headers: Vec<(&'static str, String)>,
        timeout_ms: u32,
    }

    /// Aborts its request when dropped, so a stopped or finished stream frees the connection.
    pub struct Response {
        response: web_sys::Response,
        reader: Option<web_sys::ReadableStreamDefaultReader>,
        abort: web_sys::AbortController,
    }

    impl Client {
        pub fn new(options: &Options<'_>) -> Result<Self> {
            if options.trust_ca_pem.is_some() {
                return Err(format!(
                    "{} trust certificates need the companion",
                    options.provider
                ));
            }
            let headers = options.headers.iter().chain(options.secret.iter());
            let mut headers: Vec<_> = headers
                .map(|&(name, value)| (name, value.to_owned()))
                .collect();
            headers.push(("content-type", "application/json".into()));
            headers.push(("accept", "text/event-stream".into()));
            Ok(Self {
                headers,
                timeout_ms: u32::from(options.timeout_secs) * 1000,
            })
        }

        pub async fn post(
            &self,
            url: &str,
            body: Vec<u8>,
        ) -> std::result::Result<Response, JsValue> {
            let abort = web_sys::AbortController::new()?;
            let signals = js_sys::Array::of2(
                &abort.signal(),
                &web_sys::AbortSignal::timeout_with_u32(self.timeout_ms),
            );
            let headers = web_sys::Headers::new()?;
            for (name, value) in &self.headers {
                headers.set(name, value)?;
            }
            let init = web_sys::RequestInit::new();
            init.set_method("POST");
            init.set_headers(&headers);
            init.set_body(&js_sys::Uint8Array::from(body.as_slice()));
            init.set_signal(Some(&web_sys::AbortSignal::any(&signals)));
            init.set_redirect(web_sys::RequestRedirect::Manual);
            init.set_credentials(web_sys::RequestCredentials::Omit);
            init.set_cache(web_sys::RequestCache::NoStore);
            init.set_referrer_policy(web_sys::ReferrerPolicy::NoReferrer);
            let response = JsFuture::from(fetch(url, &init)).await?.dyn_into()?;
            Ok(Response {
                response,
                reader: None,
                abort,
            })
        }
    }

    impl Response {
        pub fn status(&self) -> u16 {
            self.response.status()
        }

        pub fn content_type(&self) -> Option<String> {
            self.response.headers().get("content-type").ok().flatten()
        }

        pub async fn chunk(&mut self) -> std::result::Result<Option<Vec<u8>>, JsValue> {
            if self.reader.is_none() {
                let Some(body) = self.response.body() else {
                    return Ok(None);
                };
                self.reader = Some(body.get_reader().dyn_into()?);
            }
            let reader = self.reader.as_ref().expect("the reader was just set");
            let read = JsFuture::from(reader.read()).await?;
            if js_sys::Reflect::get(&read, &"done".into())?.is_truthy() {
                return Ok(None);
            }
            let value: js_sys::Uint8Array =
                js_sys::Reflect::get(&read, &"value".into())?.dyn_into()?;
            Ok(Some(value.to_vec()))
        }
    }

    impl Drop for Response {
        fn drop(&mut self) {
            self.abort.abort();
        }
    }

    pub async fn sleep_ms(ms: i32) {
        let wait = js_sys::Promise::new(&mut |resolve, _| {
            set_timeout(&resolve, ms);
        });
        let _ = JsFuture::from(wait).await;
    }
}

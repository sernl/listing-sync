//! A browser request shaped like a transport send, and satisfying its bound.
//!
//! This is the shell's demonstration of [`crate::bridge`], not a marketplace
//! client: it names no host, sets no marketplace header and carries no
//! session. What it shows is that a `fetch`, whose `JsFuture` is `!Send`, can
//! be handed to a caller that requires `+ Send`.
//!
//! The caller supplies the URL, so nothing here decides which origin is
//! reachable; `host_permissions` in `manifest.json` does, and it enumerates
//! them.

use crate::bridge::{bridge, Cancelled};
use core::future::Future;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{spawn_local, JsFuture};
use web_sys::{Request, RequestInit, Response};

/// Why a browser request did not produce a body.
///
/// A `String` rather than the `JsValue` it came from, because `JsValue` is
/// `!Send` and returning one would put the boundary back on the far side of
/// the bridge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchFailed(pub String);

fn describe(value: &JsValue) -> FetchFailed {
    FetchFailed(value.as_string().unwrap_or_else(|| format!("{value:?}")))
}

/// Fetches `url` and returns its body as text.
///
/// The returned future is `Send`, which is the whole point: the `!Send` work
/// happens inside [`bridge`] on the thread that started it.
pub fn fetch_text(url: &str) -> impl Future<Output = Result<String, FetchFailed>> + Send {
    let url = url.to_owned();
    let answered = bridge(spawn_local, async move { fetch_text_local(&url).await });
    async move {
        match answered.await {
            Ok(outcome) => outcome,
            Err(Cancelled) => Err(FetchFailed(
                "the browser dropped the request before it answered".to_owned(),
            )),
        }
    }
}

async fn fetch_text_local(url: &str) -> Result<String, FetchFailed> {
    let init = RequestInit::new();
    init.set_method("GET");
    let request = Request::new_with_str_and_init(url, &init).map_err(|value| describe(&value))?;

    let promise = call_fetch(&request)?;
    let response: Response = JsFuture::from(promise)
        .await
        .map_err(|value| describe(&value))?
        .dyn_into()
        .map_err(|value| describe(&value))?;

    let body = response.text().map_err(|value| describe(&value))?;
    JsFuture::from(body)
        .await
        .map_err(|value| describe(&value))?
        .as_string()
        .ok_or_else(|| FetchFailed("the response body was not a string".to_owned()))
}

/// Calls the global `fetch`.
///
/// Reached through the global object rather than through `Window` or
/// `WorkerGlobalScope` because the shell instantiates the same module in a
/// service worker and in the popup page, and those are different globals.
fn call_fetch(request: &Request) -> Result<js_sys::Promise, FetchFailed> {
    let global = js_sys::global();
    let fetch: js_sys::Function = js_sys::Reflect::get(&global, &JsValue::from_str("fetch"))
        .map_err(|value| describe(&value))?
        .dyn_into()
        .map_err(|value| describe(&value))?;

    fetch
        .call1(&global, request)
        .map_err(|value| describe(&value))?
        .dyn_into()
        .map_err(|value| describe(&value))
}

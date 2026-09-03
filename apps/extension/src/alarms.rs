//! `chrome.alarms`, hand-declared.
//!
//! No Rust bindings crate covers this namespace: oxichrome lacks it and so
//! does `web-extensions-sys`, which is the crate the memo otherwise picks for
//! the namespaces it does cover. The block below is the entire cost of that
//! gap, which is why the bindings-crate question is a minor one.
//!
//! Alarms are the scheduler's only timer, because they are the only API that
//! can wake a terminated MV3 service worker. Chrome's 30-second minimum period
//! never binds on this product: the default cadence is one hour.

use wasm_bindgen::prelude::{wasm_bindgen, Closure};
use wasm_bindgen::{JsCast, JsValue};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["chrome", "alarms"], js_name = create)]
    pub fn create(name: &str, info: &JsValue);

    #[wasm_bindgen(js_namespace = ["chrome", "alarms"], js_name = clear)]
    pub fn clear(name: &str) -> js_sys::Promise;

    #[wasm_bindgen(js_namespace = ["chrome", "alarms", "onAlarm"], js_name = addListener)]
    pub fn add_alarm_listener(handler: &Closure<dyn FnMut(JsValue)>);
}

/// Creates a repeating alarm whose period is stated in minutes.
///
/// The period is `f64` because that is what the API reads; a caller stating a
/// cadence in hours converts at the call site rather than here.
pub fn create_periodic(name: &str, period_minutes: f64) -> Result<(), JsValue> {
    let info = js_sys::Object::new();
    js_sys::Reflect::set(
        &info,
        &JsValue::from_str("periodInMinutes"),
        &JsValue::from_f64(period_minutes),
    )?;
    create(name, info.unchecked_ref());
    Ok(())
}

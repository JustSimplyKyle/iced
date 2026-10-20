//! URL-backed SVGs share one download per URL across widget rebuilds.

use crate::core::svg::Handle;
use js_sys::Uint8Array;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::{JsFuture, spawn_local};

thread_local! {
    static CACHE: RefCell<HashMap<String, Source>> = RefCell::new(HashMap::new());
}

#[derive(Clone)]
pub(super) struct Source(Rc<RefCell<State>>);

struct State {
    pending: bool,
    handle: Option<Handle>,
}

impl Source {
    pub(super) fn new(url: String) -> Self {
        CACHE.with(|cache| {
            cache
                .borrow_mut()
                .entry(url.clone())
                .or_insert_with(|| {
                    let source = Self(Rc::new(RefCell::new(State {
                        pending: true,
                        handle: None,
                    })));
                    let loading = source.clone();
                    spawn_local(async move {
                        let result = Self::fetch(&url).await;
                        let mut state = loading.0.borrow_mut();
                        state.pending = false;
                        match result {
                            Ok(handle) => state.handle = Some(handle),
                            Err(error) => log::warn!("Cannot load SVG {url}: {error}"),
                        }
                    });
                    source
                })
                .clone()
        })
    }

    pub(super) fn pending(&self) -> bool {
        self.0.borrow().pending
    }

    pub(super) fn handle(&self) -> Option<Handle> {
        self.0.borrow().handle.clone()
    }

    async fn fetch(url: &str) -> Result<Handle, String> {
        let window = web_sys::window().ok_or("browser window is unavailable")?;
        let response = JsFuture::from(window.fetch_with_str(url))
            .await
            .map_err(|error| format!("request failed: {error:?}"))?
            .dyn_into::<web_sys::Response>()
            .map_err(|_| "fetch did not return a Response")?;
        if !response.ok() {
            return Err(format!("HTTP {}", response.status()));
        }
        let buffer = JsFuture::from(
            response
                .array_buffer()
                .map_err(|error| format!("cannot read response: {error:?}"))?,
        )
        .await
        .map_err(|error| format!("cannot read SVG bytes: {error:?}"))?;
        Ok(Handle::from_memory(Uint8Array::new(&buffer).to_vec()))
    }
}

//! The model store: each model the extension runs on its own is downloaded once from a pinned
//! Hugging Face revision, checked by SHA-256 and kept in IndexedDB (compressed ones stay
//! compressed and expand on load). A new version of a model replaces its old one in the same write.

use super::*;
use web_sys::{IdbDatabase, IdbOpenDbRequest, IdbTransactionMode};

const DATABASE: &str = "metldr-models";
const STORE: &str = "files";

/// One model at a pinned revision.
pub(crate) struct Pinned {
    /// The model, as its keys start; one version of it is kept.
    pub name: &'static str,
    pub version: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    /// The file is gzip, expanded by the browser on load.
    pub gzip: bool,
}

/// The search model (see `crate::dense`): blink-8M as int8, chosen on four retrieval benchmarks.
pub(crate) const SEARCH: Pinned = Pinned {
    name: "search",
    version: "blink-8M-int8@2358eb5a",
    url: "https://huggingface.co/WhiteHades/blink-8M-int8/resolve/2358eb5a8035a4e87f66e32024824f7481dbca88/blink-8M-int8.metldr",
    sha256: "a099b59e55dfd91474e3979c05727c2c8a6cc622a83907f6d7252b06b91a04a2",
    gzip: false,
};

/// The language detector (see `language.rs`): py3langid in compact form.
pub(crate) const LANGUAGES: Pinned = Pinned {
    name: "languages",
    version: "py3langid-compact@21f3535a",
    url: "https://huggingface.co/WhiteHades/py3langid-compact/resolve/21f3535ae230fc811e02a3b72696417d7dd3d866/py3langid-compact.lidc.gz",
    sha256: "f76f8dab751f43eeea3877784629b000c87bb49dceaa61ffa21010a80681d3a2",
    gzip: true,
};

thread_local! {
    static SEARCH_READY: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

/// Whether the search model is ready to take. False offline or on a mismatch; search then runs
/// on keywords alone.
pub(super) async fn search_model_fetch() -> bool {
    if SEARCH_READY.with_borrow(Option::is_some) {
        return true;
    }
    let bytes = fetch_pinned(&SEARCH).await;
    let ready = bytes.is_some();
    SEARCH_READY.with_borrow_mut(|slot| *slot = bytes);
    ready
}

/// The fetched search model's bytes, once.
pub(crate) fn take_search_model() -> Option<Vec<u8>> {
    SEARCH_READY.with_borrow_mut(Option::take)
}

/// `model`'s bytes: from IndexedDB, else downloaded, checked and kept.
pub(crate) async fn fetch_pinned(model: &Pinned) -> Option<Vec<u8>> {
    let db = database().await.ok()?;
    let key = format!("{}:{}", model.name, model.version);
    let kept = match stored(&db, &key).await {
        Some(bytes) => Some(bytes),
        None => match download(model).await {
            Ok(bytes) => {
                let _ = store(&db, model.name, &key, &bytes).await;
                Some(bytes)
            }
            Err(_) => None,
        },
    };
    db.close();
    let kept = kept?;
    if model.gzip {
        gunzip(&kept).await.ok()
    } else {
        Some(kept)
    }
}

async fn download(model: &Pinned) -> Result<Vec<u8>, Error> {
    let init = web_sys::RequestInit::new();
    init.set_credentials(web_sys::RequestCredentials::Omit);
    let response: web_sys::Response = JsFuture::from(fetch(model.url, &init))
        .await
        .map_err(err)?
        .unchecked_into();
    if !response.ok() {
        return Err(format!("A model could not download: {}", response.status()));
    }
    let buffer = JsFuture::from(response.array_buffer().map_err(err)?)
        .await
        .map_err(err)?;
    if subtle_digest(&buffer).await? != model.sha256 {
        return Err("A model did not match its pinned hash.".into());
    }
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// `bytes` expanded by the browser's own gzip decoder.
async fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let global = js_sys::global();
    let construct = |name: &str, args: &js_sys::Array| -> Result<JsValue, Error> {
        let class: js_sys::Function = get(&global, name)
            .dyn_into()
            .map_err(|_| format!("No {name} here."))?;
        js_sys::Reflect::construct(&class, args).map_err(err)
    };
    let source = construct(
        "Response",
        &js_sys::Array::of1(&js_sys::Uint8Array::from(bytes)),
    )?;
    let decoder = construct("DecompressionStream", &js_sys::Array::of1(&"gzip".into()))?;
    let body = get(&source, "body");
    let pipe: js_sys::Function = get(&body, "pipeThrough")
        .dyn_into()
        .map_err(|_| "No streams here.")?;
    let piped = pipe.call1(&body, &decoder).map_err(err)?;
    let expanded: web_sys::Response =
        construct("Response", &js_sys::Array::of1(&piped))?.unchecked_into();
    let buffer = JsFuture::from(expanded.array_buffer().map_err(err)?)
        .await
        .map_err(err)?;
    Ok(js_sys::Uint8Array::new(&buffer).to_vec())
}

/// The SHA-256 of `buffer` in lowercase hex, by WebCrypto.
async fn subtle_digest(buffer: &JsValue) -> Result<String, Error> {
    let crypto: web_sys::Crypto = get(&js_sys::global(), "crypto")
        .dyn_into()
        .map_err(|_| "No WebCrypto here.")?;
    let object: js_sys::Object = buffer.clone().unchecked_into();
    let digest = crypto
        .subtle()
        .digest_with_str_and_buffer_source("SHA-256", &object)
        .map_err(err)?;
    let digest = js_sys::Uint8Array::new(&JsFuture::from(digest).await.map_err(err)?).to_vec();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

async fn stored(db: &IdbDatabase, key: &str) -> Option<Vec<u8>> {
    let store = db
        .transaction_with_str(STORE)
        .and_then(|tx| tx.object_store(STORE))
        .ok()?;
    let found = keys::done(&store.get(&key.into()).ok()?).await.ok()?;
    found
        .dyn_ref::<js_sys::Uint8Array>()
        .map(js_sys::Uint8Array::to_vec)
}

/// Keeps `bytes` under `key`, removing any other version of model `name` in the same write.
async fn store(db: &IdbDatabase, name: &str, key: &str, bytes: &[u8]) -> Result<(), Error> {
    let tx = db
        .transaction_with_str_and_mode(STORE, IdbTransactionMode::Readwrite)
        .map_err(err)?;
    let files = tx.object_store(STORE).map_err(err)?;
    // Every key of this model (`name:` up to `name;`), the old versions among them.
    let versions =
        web_sys::IdbKeyRange::bound(&format!("{name}:").into(), &format!("{name};").into())
            .map_err(err)?;
    files.delete(&versions).map_err(err)?;
    files
        .put_with_key(&js_sys::Uint8Array::from(bytes), &key.into())
        .map_err(err)?;
    dictionary::finished(&tx).await
}

async fn database() -> Result<IdbDatabase, Error> {
    let factory: web_sys::IdbFactory = get(&js_sys::global(), "indexedDB")
        .dyn_into()
        .map_err(|_| "No IndexedDB here.")?;
    let open: IdbOpenDbRequest = factory.open_with_u32(DATABASE, 1).map_err(err)?;
    let upgrade = Closure::once_into_js({
        let open = open.clone();
        move || {
            if let Some(db) = open
                .result()
                .ok()
                .and_then(|r| r.dyn_into::<IdbDatabase>().ok())
                && !db.object_store_names().contains(STORE)
            {
                let _ = db.create_object_store(STORE);
            }
        }
    });
    open.set_onupgradeneeded(Some(upgrade.unchecked_ref()));
    keys::done(&open)
        .await?
        .dyn_into()
        .map_err(|_| "The model store could not be opened.".into())
}

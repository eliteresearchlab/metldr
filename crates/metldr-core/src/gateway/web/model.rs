//! The search model, downloaded once from a pinned Hugging Face revision, checked by SHA-256 and
//! kept in IndexedDB; the bytes stay in this module for the search index to take.

use super::*;
use web_sys::{IdbDatabase, IdbOpenDbRequest, IdbTransactionMode};

const URL: &str = "https://huggingface.co/WhiteHades/blink-8M-int8/resolve/2358eb5a8035a4e87f66e32024824f7481dbca88/blink-8M-int8.metldr";
const SHA256: &str = "a099b59e55dfd91474e3979c05727c2c8a6cc622a83907f6d7252b06b91a04a2";
const DATABASE: &str = "metldr-models";
const STORE: &str = "files";
const KEY: &str = "blink-8M-int8@2358eb5a";

thread_local! {
    static READY: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

/// Whether the search model is ready to take: read from IndexedDB, else downloaded, checked and
/// stored. False offline or on a mismatch; search then runs on keywords alone.
#[wasm_bindgen]
pub async fn search_model_fetch() -> bool {
    if READY.with_borrow(Option::is_some) {
        return true;
    }
    let Ok(db) = database().await else {
        return false;
    };
    let bytes = match stored(&db).await {
        Some(bytes) => Some(bytes),
        None => match download().await {
            Ok(bytes) => {
                let _ = store(&db, &bytes).await;
                Some(bytes)
            }
            Err(_) => None,
        },
    };
    db.close();
    let ready = bytes.is_some();
    READY.with_borrow_mut(|slot| *slot = bytes);
    ready
}

/// The fetched model's bytes, once.
pub(crate) fn take_search_model() -> Option<Vec<u8>> {
    READY.with_borrow_mut(Option::take)
}

async fn download() -> Result<Vec<u8>, Error> {
    let init = web_sys::RequestInit::new();
    init.set_credentials(web_sys::RequestCredentials::Omit);
    let response: web_sys::Response = JsFuture::from(fetch(URL, &init))
        .await
        .map_err(err)?
        .unchecked_into();
    if !response.ok() {
        return Err(format!(
            "The search model could not download: {}",
            response.status()
        ));
    }
    let buffer = JsFuture::from(response.array_buffer().map_err(err)?)
        .await
        .map_err(err)?;
    let digest = subtle_digest(&buffer).await?;
    if digest != SHA256 {
        return Err("The search model did not match its pinned hash.".into());
    }
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

async fn stored(db: &IdbDatabase) -> Option<Vec<u8>> {
    let store = db
        .transaction_with_str(STORE)
        .and_then(|tx| tx.object_store(STORE))
        .ok()?;
    let found = keys::done(&store.get(&KEY.into()).ok()?).await.ok()?;
    found
        .dyn_ref::<js_sys::Uint8Array>()
        .map(js_sys::Uint8Array::to_vec)
}

async fn store(db: &IdbDatabase, bytes: &[u8]) -> Result<(), Error> {
    let tx = db
        .transaction_with_str_and_mode(STORE, IdbTransactionMode::Readwrite)
        .map_err(err)?;
    let value = js_sys::Uint8Array::from(bytes);
    let files = tx.object_store(STORE).map_err(err)?;
    // One model kept: a replaced one's bytes go in the same write.
    files.clear().map_err(err)?;
    files.put_with_key(&value, &KEY.into()).map_err(err)?;
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

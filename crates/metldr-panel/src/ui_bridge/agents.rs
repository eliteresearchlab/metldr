use super::settings::{Family, Work};
use super::*;
use crate::model::{AgentSettingState, AgentSettingView};
use std::future::Future;
use wasm_bindgen_futures::future_to_promise_typed;

mod profile;
pub use profile::UiAgentProfile;

#[wasm_bindgen]
pub struct UiAgentSettings {
    pub(super) page: Rc<PageSession>,
}

impl UiAgentSettings {
    pub(super) fn new(page: Rc<PageSession>) -> Self {
        Self { page }
    }
    pub(super) fn refresh_work(
        &self,
    ) -> Result<impl Future<Output = Result<js_sys::Undefined, FlowError>> + 'static, FlowError>
    {
        let work = Work::begin(Rc::clone(&self.page), Family::Agents, true)?;
        Ok(async move { work.refresh().await })
    }
}

#[wasm_bindgen]
impl UiAgentSettings {
    pub fn configurations(&self) -> Vec<UiAgentConfiguration> {
        self.page
            .view
            .borrow()
            .agent_settings
            .agents
            .iter()
            .map(|row| UiAgentConfiguration::new(Rc::clone(&self.page), row.clone()))
            .collect()
    }

    pub fn configuration(
        &self,
        agent: String,
        revision: u64,
    ) -> Result<UiAgentConfiguration, JsValue> {
        self.page
            .view
            .borrow()
            .agent_settings
            .agents
            .iter()
            .find(|row| row.agent.name() == agent && row.revision == revision)
            .map(|row| UiAgentConfiguration::new(Rc::clone(&self.page), row.clone()))
            .ok_or_else(|| js_error(FlowError::HistoryStale))
    }

    #[wasm_bindgen(unchecked_return_type = "Promise<undefined>")]
    pub fn refresh(&self) -> Result<Promise<js_sys::Undefined>, JsValue> {
        let work = self.refresh_work().map_err(js_error)?;
        Ok(future_to_promise_typed(async move {
            work.await.map_err(js_error)
        }))
    }
}

#[wasm_bindgen]
pub struct UiAgentConfiguration {
    page: Rc<PageSession>,
    row: AgentSettingView,
}

impl UiAgentConfiguration {
    pub(super) fn new(page: Rc<PageSession>, row: AgentSettingView) -> Self {
        Self { page, row }
    }

    fn begin(&self) -> Result<Work, FlowError> {
        if !self
            .page
            .view
            .borrow()
            .agent_settings
            .agents
            .iter()
            .any(|row| row.agent == self.row.agent && row.revision == self.row.revision)
        {
            return Err(FlowError::HistoryStale);
        }
        Work::begin(Rc::clone(&self.page), Family::Agents, false)
    }

    pub(super) fn save_work(
        &self,
        profile: crate::model::AgentProfile,
    ) -> Result<impl Future<Output = Result<js_sys::Undefined, FlowError>> + 'static, FlowError>
    {
        if profile.agent() != self.row.agent || !crate::model::valid_agent_profile(&profile) {
            return Err(FlowError::InvalidInput(
                "Agent paths or model settings are invalid or exceed their limits.".into(),
            ));
        }
        let work = self.begin()?;
        let revision = self.row.revision;
        Ok(async move {
            let result = async {
                work.current_flow()
                    .await?
                    .agent_save(revision, profile)
                    .await
            }
            .await;
            work.finish(result)
        })
    }

    pub(super) fn remove_work(
        &self,
    ) -> Result<impl Future<Output = Result<js_sys::Undefined, FlowError>> + 'static, FlowError>
    {
        let work = self.begin()?;
        let agent = self.row.agent;
        let revision = self.row.revision;
        Ok(async move {
            let result = async {
                work.current_flow()
                    .await?
                    .agent_remove(agent, revision)
                    .await
            }
            .await;
            work.finish(result)
        })
    }
}

#[wasm_bindgen]
impl UiAgentConfiguration {
    #[wasm_bindgen(getter)]
    pub fn agent(&self) -> String {
        self.row.agent.name().into()
    }
    #[wasm_bindgen(getter)]
    pub fn label(&self) -> String {
        self.row.agent.label().into()
    }
    #[wasm_bindgen(getter)]
    pub fn revision(&self) -> u64 {
        self.row.revision
    }
    #[wasm_bindgen(getter)]
    pub fn state(&self) -> String {
        match self.row.state {
            AgentSettingState::Unmanaged => "unmanaged",
            AgentSettingState::Ready => "ready",
            AgentSettingState::Removed => "removed",
            AgentSettingState::Unavailable => "unavailable",
        }
        .into()
    }
    pub fn profile(&self) -> Option<UiAgentProfile> {
        self.row.profile.clone().map(UiAgentProfile::from_profile)
    }

    #[wasm_bindgen(unchecked_return_type = "Promise<undefined>")]
    pub fn save(&self, profile: &UiAgentProfile) -> Result<Promise<js_sys::Undefined>, JsValue> {
        if profile.profile.agent() != self.row.agent {
            return Err(crate::err(
                "The profile belongs to another installed agent.",
            ));
        }
        let work = self.save_work(profile.profile.clone()).map_err(js_error)?;
        Ok(future_to_promise_typed(async move {
            work.await.map_err(js_error)
        }))
    }

    #[wasm_bindgen(unchecked_return_type = "Promise<undefined>")]
    pub fn remove(&self) -> Result<Promise<js_sys::Undefined>, JsValue> {
        let work = self.remove_work().map_err(js_error)?;
        Ok(future_to_promise_typed(async move {
            work.await.map_err(js_error)
        }))
    }
}

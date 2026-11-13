use super::*;
use crate::model::{AgentKind, AgentProfile, ProviderProtocol};
use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    rc::{Rc, Weak},
};

type WorkFuture = Pin<Box<dyn Future<Output = Result<js_sys::Undefined, FlowError>>>>;

struct ProviderDraft {
    protocol: ProviderProtocol,
    revision: u64,
    endpoint: TextEdit,
    model: TextEdit,
    key: TextEdit,
    /// Chat Completions only.
    keyless: bool,
    timeout: TextEdit,
    dirty: bool,
    version: u64,
}

struct AgentDraft {
    agent: AgentKind,
    revision: u64,
    executable: TextEdit,
    workspace: TextEdit,
    session_dir: TextEdit,
    provider: TextEdit,
    model: TextEdit,
    offline: bool,
    dirty: bool,
    version: u64,
}

#[derive(Clone, Copy)]
enum Target {
    Provider(usize),
    Agent(usize),
    Refresh,
}

struct Completed {
    target: Target,
    version: u64,
    success: bool,
}

pub(super) struct Settings {
    owner: Weak<Core>,
    flow: Option<FlowClient>,
    providers: Vec<ProviderDraft>,
    agents: Vec<AgentDraft>,
    completed: Rc<RefCell<Option<Completed>>>,
    message: Option<&'static str>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            owner: Weak::new(),
            flow: None,
            providers: ProviderProtocol::ALL
                .into_iter()
                .map(|protocol| ProviderDraft {
                    protocol,
                    revision: 0,
                    endpoint: TextEdit::default(),
                    model: TextEdit::default(),
                    key: TextEdit::default().secure(),
                    keyless: false,
                    timeout: TextEdit::default(),
                    dirty: false,
                    version: 0,
                })
                .collect(),
            agents: AgentKind::ALL
                .into_iter()
                .map(|agent| AgentDraft {
                    agent,
                    revision: 0,
                    executable: TextEdit::default(),
                    workspace: TextEdit::default(),
                    session_dir: TextEdit::default(),
                    provider: TextEdit::default(),
                    model: TextEdit::default(),
                    offline: false,
                    dirty: false,
                    version: 0,
                })
                .collect(),
            completed: Rc::new(RefCell::new(None)),
            message: None,
        }
    }
}

impl Settings {
    /// Call from the shared poll after accepting page updates.
    pub(super) fn sync(&mut self, core: &Rc<Core>) {
        if self.owner.as_ptr() != Rc::as_ptr(core) {
            *self = Self::default();
            self.owner = Rc::downgrade(core);
        }
        let flow = core.page.flow.borrow().clone();
        if core.page.invalidated.get()
            || !flow.is_connected()
            || self
                .flow
                .as_ref()
                .is_none_or(|previous| !flow.same_connection(previous))
        {
            self.clear_keys();
        }
        self.flow = Some(flow);
        let completed = self.completed.borrow_mut().take();
        if let Some(completed) = completed {
            self.message = if completed.success {
                None
            } else {
                Some(
                    "Settings could not be updated. Refresh before retrying if the result is uncertain.",
                )
            };
            if completed.success {
                match completed.target {
                    Target::Provider(i) => {
                        if let Some(d) = self.providers.get_mut(i)
                            && d.version == completed.version
                        {
                            d.dirty = false;
                        }
                    }
                    Target::Agent(i) => {
                        if let Some(d) = self.agents.get_mut(i)
                            && d.version == completed.version
                        {
                            d.dirty = false;
                        }
                    }
                    Target::Refresh => {}
                }
            }
        }
        let view = core.page.view.borrow();
        for draft in &mut self.providers {
            if !draft.dirty
                && let Some(row) = view
                    .provider_settings
                    .connections
                    .iter()
                    .find(|row| row.protocol == draft.protocol)
            {
                draft.revision = row.revision;
                replace_if_changed(&mut draft.endpoint, row.endpoint.as_deref().unwrap_or(""));
                replace_if_changed(&mut draft.model, row.model.as_deref().unwrap_or(""));
                draft.keyless = row.keyless;
                replace_if_changed(
                    &mut draft.timeout,
                    &row.timeout_secs
                        .map(|secs| secs.to_string())
                        .unwrap_or_default(),
                );
            }
        }
        for draft in &mut self.agents {
            if !draft.dirty
                && let Some(row) = view
                    .agent_settings
                    .agents
                    .iter()
                    .find(|row| row.agent == draft.agent)
            {
                draft.revision = row.revision;
                draft.load(row.profile.as_ref());
            }
        }
    }

    pub(super) fn clear_keys(&mut self) {
        for draft in &mut self.providers {
            draft.key.clear();
        }
    }

    fn launch(
        &mut self,
        target: Target,
        version: u64,
        work: Result<WorkFuture, FlowError>,
        updates: &Rc<Updates>,
    ) {
        match work {
            Ok(work) => {
                self.message = None;
                let completed = Rc::downgrade(&self.completed);
                let updates = Rc::downgrade(updates);
                spawn_local(async move {
                    let success = work.await.is_ok();
                    if let Some(completed) = completed.upgrade() {
                        *completed.borrow_mut() = Some(Completed {
                            target,
                            version,
                            success,
                        });
                    }
                    if let Some(updates) = updates.upgrade() {
                        updates.changed();
                    }
                });
            }
            Err(_) => {
                self.message =
                    Some("Settings changed or are busy. Review and refresh before retrying.")
            }
        }
    }

    fn provider_action(state: &mut State, index: usize, expected_version: u64, save: bool) {
        let (Some(core), Some(updates)) = (state.core().cloned(), state.updates.clone()) else {
            return;
        };
        let Some(draft) = state.settings.providers.get_mut(index) else {
            return;
        };
        if draft.version != expected_version {
            return;
        }
        if save
            && [&draft.endpoint, &draft.model, &draft.key, &draft.timeout]
                .iter()
                .any(|edit| edit.preedit.is_some())
        {
            state.settings.message = Some("Finish composing text before saving.");
            return;
        }
        let chat = draft.protocol == ProviderProtocol::ChatCompletions;
        let timeout = match draft.timeout.text.trim() {
            "" => None,
            text => match text.parse::<u16>() {
                Ok(secs) if chat && crate::model::PROVIDER_TIMEOUT_SECS.contains(&secs) => {
                    Some(secs)
                }
                _ if save => {
                    state.settings.message = Some("Response timeout must be 5 to 600 seconds.");
                    return;
                }
                _ => None,
            },
        };
        let row = core
            .page
            .view
            .borrow()
            .provider_settings
            .connections
            .iter()
            .find(|row| row.protocol == draft.protocol && row.revision == draft.revision)
            .cloned();
        if save
            && chat
            && !draft.keyless
            && draft.key.text.is_empty()
            && row.as_ref().is_some_and(|row| row.keyless)
        {
            state.settings.message = Some("Enter an API key.");
            return;
        }
        // Send the timeout only when it changed, so an older companion still accepts other saves.
        let saved_timeout = row.as_ref().and_then(|row| row.timeout_secs);
        let timeout = match (timeout, saved_timeout) {
            (requested, saved) if requested == saved => None,
            (None, Some(_)) => Some(0),
            (requested, _) => requested,
        };
        let key = if save && row.is_some() && chat && draft.keyless {
            crate::model::ProviderKey::None
        } else if save && row.is_some() && !draft.key.text.is_empty() {
            crate::model::ProviderKey::Replace(std::mem::take(&mut draft.key.text))
        } else {
            crate::model::ProviderKey::Keep
        };
        draft.key.clear();
        let version = draft.version;
        let work: Result<WorkFuture, FlowError> = match row {
            Some(row) => {
                let connection = UiProviderConnection::new(Rc::clone(&core.page), row);
                if save {
                    connection
                        .save_work(
                            draft.endpoint.text.clone(),
                            draft.model.text.clone(),
                            key,
                            timeout,
                        )
                        .map(|work| Box::pin(work) as WorkFuture)
                } else {
                    connection
                        .remove_work()
                        .map(|work| Box::pin(work) as WorkFuture)
                }
            }
            None => Err(FlowError::HistoryStale),
        };
        state
            .settings
            .launch(Target::Provider(index), version, work, &updates);
    }

    /// Fills the Chat Completions endpoint from a known provider; the user still saves.
    fn chat_preset(state: &mut State, index: usize, expected_version: u64, preset: usize) {
        let Some(core) = state.core().cloned() else {
            return;
        };
        if !core.page.controls_ready() || core.page.settings_blocked() {
            return;
        }
        let (Some(draft), Some((_, endpoint, keyless))) = (
            state.settings.providers.get_mut(index),
            crate::model::CHAT_PRESETS.get(preset),
        ) else {
            return;
        };
        if draft.version != expected_version {
            return;
        }
        replace_if_changed(&mut draft.endpoint, endpoint);
        draft.keyless = *keyless;
        draft.key.clear();
        draft.dirty = true;
        draft.version = draft.version.wrapping_add(1);
    }

    fn agent_action(state: &mut State, index: usize, expected_version: u64, save: bool) {
        let (Some(core), Some(updates)) = (state.core().cloned(), state.updates.clone()) else {
            return;
        };
        let Some(draft) = state.settings.agents.get(index) else {
            return;
        };
        if draft.version != expected_version {
            return;
        }
        if save
            && [
                &draft.executable,
                &draft.workspace,
                &draft.session_dir,
                &draft.provider,
                &draft.model,
            ]
            .iter()
            .any(|edit| edit.preedit.is_some())
        {
            state.settings.message = Some("Finish composing text before saving.");
            return;
        }
        let row = core
            .page
            .view
            .borrow()
            .agent_settings
            .agents
            .iter()
            .find(|row| row.agent == draft.agent && row.revision == draft.revision)
            .cloned();
        let version = draft.version;
        let work: Result<WorkFuture, FlowError> = match row {
            Some(row) => {
                let configuration = UiAgentConfiguration::new(Rc::clone(&core.page), row);
                if save {
                    configuration
                        .save_work(draft.profile())
                        .map(|work| Box::pin(work) as WorkFuture)
                } else {
                    configuration
                        .remove_work()
                        .map(|work| Box::pin(work) as WorkFuture)
                }
            }
            None => Err(FlowError::HistoryStale),
        };
        state
            .settings
            .launch(Target::Agent(index), version, work, &updates);
    }

    fn refresh(state: &mut State, family: crate::ui_bridge::settings::Family) {
        let (Some(core), Some(updates)) = (state.core().cloned(), state.updates.clone()) else {
            return;
        };
        let work: Result<WorkFuture, FlowError> = match family {
            crate::ui_bridge::settings::Family::Providers => {
                UiProviderSettings::new(Rc::clone(&core.page))
                    .refresh_work()
                    .map(|work| Box::pin(work) as WorkFuture)
            }
            crate::ui_bridge::settings::Family::Agents => {
                UiAgentSettings::new(Rc::clone(&core.page))
                    .refresh_work()
                    .map(|work| Box::pin(work) as WorkFuture)
            }
        };
        state.settings.launch(Target::Refresh, 0, work, &updates);
    }
}

impl AgentDraft {
    fn load(&mut self, profile: Option<&AgentProfile>) {
        let (executable, workspace, session_dir, provider, model, offline) = match profile {
            Some(
                AgentProfile::Codex {
                    executable,
                    workspace,
                }
                | AgentProfile::Opencode {
                    executable,
                    workspace,
                },
            ) => (executable.as_str(), workspace.as_str(), "", "", "", false),
            Some(AgentProfile::Pi {
                executable,
                workspace,
                session_dir,
                provider,
                model,
                offline,
            }) => (
                executable.as_str(),
                workspace.as_str(),
                session_dir.as_str(),
                provider.as_str(),
                model.as_str(),
                *offline,
            ),
            None => ("", "", "", "", "", false),
        };
        replace_if_changed(&mut self.executable, executable);
        replace_if_changed(&mut self.workspace, workspace);
        replace_if_changed(&mut self.session_dir, session_dir);
        replace_if_changed(&mut self.provider, provider);
        replace_if_changed(&mut self.model, model);
        self.offline = offline;
    }
    fn profile(&self) -> AgentProfile {
        match self.agent {
            AgentKind::Codex => AgentProfile::Codex {
                executable: self.executable.text.clone(),
                workspace: self.workspace.text.clone(),
            },
            AgentKind::Opencode => AgentProfile::Opencode {
                executable: self.executable.text.clone(),
                workspace: self.workspace.text.clone(),
            },
            AgentKind::Pi => AgentProfile::Pi {
                executable: self.executable.text.clone(),
                workspace: self.workspace.text.clone(),
                session_dir: self.session_dir.text.clone(),
                provider: self.provider.text.clone(),
                model: self.model.text.clone(),
                offline: self.offline,
            },
        }
    }
}

fn replace_if_changed(edit: &mut TextEdit, value: &str) {
    if edit.text != value {
        edit.replace_text(value);
    }
}

fn saved_profile(profile: Option<&AgentProfile>) -> String {
    match profile {
        Some(
            AgentProfile::Codex {
                executable,
                workspace,
            }
            | AgentProfile::Opencode {
                executable,
                workspace,
            },
        ) => format!("Saved executable: {executable}. Saved workspace: {workspace}."),
        Some(AgentProfile::Pi {
            executable,
            workspace,
            session_dir,
            provider,
            model,
            offline,
        }) => format!(
            "Saved executable: {executable}. Saved workspace: {workspace}. Saved session directory: {session_dir}. Saved provider: {provider}. Saved model: {model}. Offline: {}.",
            if *offline { "on" } else { "off" }
        ),
        None => "No saved profile.".into(),
    }
}

#[derive(Clone, Copy)]
enum ProviderField {
    Endpoint,
    Model,
    Key,
    Timeout,
}
#[derive(Clone, Copy)]
enum AgentField {
    Executable,
    Workspace,
    Session,
    Provider,
    Model,
}

fn provider_input(
    index: usize,
    field: ProviderField,
    label: &'static str,
    context: &str,
    edit: &TextEdit,
    enabled: bool,
    typography: super::preferences::Typography,
    palette: super::preferences::Palette,
) -> El {
    let ink = palette.ink;
    let mut input = ::rune::el::text_input(
        edit,
        TextStyle::new((16.0 * typography.scale).max(16.0), 400.0),
    )
    .key(format!("settings-provider-{index}-{label}"))
    .label(format!("{context} {label}"))
    .w_full()
    .min_h(36.0)
    .padding(6.0)
    .single_line()
    .fill(palette.field)
    .border(1.0, palette.line)
    .color(ink)
    .caret(ink, oklch(0.68, 0.12, 250.0).alpha(0.3))
    .disabled(!enabled)
    .on_edit(move |state: &mut State| match field {
        ProviderField::Endpoint => &mut state.settings.providers[index].endpoint,
        ProviderField::Model => &mut state.settings.providers[index].model,
        ProviderField::Key => &mut state.settings.providers[index].key,
        ProviderField::Timeout => &mut state.settings.providers[index].timeout,
    })
    .on_input_change(move |state: &mut State, _| {
        let draft = &mut state.settings.providers[index];
        draft.dirty = true;
        draft.version = draft.version.wrapping_add(1);
    });
    if matches!(field, ProviderField::Key) {
        input = input
            .secure()
            .placeholder("Leave blank to keep the saved key", ink.alpha(0.55));
    }
    if matches!(field, ProviderField::Timeout) {
        input = input.placeholder("45", ink.alpha(0.55));
    }
    col().gap(3.0).child(typography.text(label)).child(input)
}

fn agent_input(
    index: usize,
    field: AgentField,
    label: &'static str,
    context: &str,
    edit: &TextEdit,
    enabled: bool,
    typography: super::preferences::Typography,
    palette: super::preferences::Palette,
) -> El {
    let ink = palette.ink;
    let input = ::rune::el::text_input(
        edit,
        TextStyle::new((16.0 * typography.scale).max(16.0), 400.0),
    )
    .key(format!("settings-agent-{index}-{label}"))
    .label(format!("{context} {label}"))
    .w_full()
    .min_h(36.0)
    .padding(6.0)
    .single_line()
    .fill(palette.field)
    .border(1.0, palette.line)
    .color(ink)
    .caret(ink, oklch(0.68, 0.12, 250.0).alpha(0.3))
    .disabled(!enabled)
    .on_edit(move |state: &mut State| match field {
        AgentField::Executable => &mut state.settings.agents[index].executable,
        AgentField::Workspace => &mut state.settings.agents[index].workspace,
        AgentField::Session => &mut state.settings.agents[index].session_dir,
        AgentField::Provider => &mut state.settings.agents[index].provider,
        AgentField::Model => &mut state.settings.agents[index].model,
    })
    .on_input_change(move |state: &mut State, _| {
        let draft = &mut state.settings.agents[index];
        draft.dirty = true;
        draft.version = draft.version.wrapping_add(1);
    });
    col().gap(3.0).child(typography.text(label)).child(input)
}

fn button(
    typography: super::preferences::Typography,
    label: &'static str,
    accessible_name: String,
    key: String,
    enabled: bool,
    action: impl Fn(&mut State) + 'static,
) -> El {
    let cue = match label {
        "Save" => crate::interaction_audio::Cue::Pulse,
        "Remove" => crate::interaction_audio::Cue::Release,
        "Offline: on" | "Offline: off" => crate::interaction_audio::Cue::Tick,
        _ => crate::interaction_audio::Cue::Press,
    };
    let element = typography
        .text(label)
        .label(accessible_name)
        .key(key)
        .role("button")
        .focusable(true)
        .disabled(!enabled)
        .padding(6.0);
    if !enabled {
        return element;
    }
    let action = Rc::new(action);
    let press = Rc::clone(&action);
    element
        .press(oklch(0.7, 0.08, 250.0).alpha(0.2))
        .on_press(move |state: &mut State| {
            state.sound(cue);
            press(state)
        })
        .on_key(move |state: &mut State, key, tx| {
            if key.key != ::rune::Key::Enter {
                return false;
            }
            if !tx.key_repeat() {
                state.sound(cue);
                action(state);
            }
            true
        })
}

impl Settings {
    /// Display inside the caller's settings route, after `sync`.
    pub(super) fn view(
        &self,
        core: &Rc<Core>,
        _cx: &mut Cx<State>,
        typography: super::preferences::Typography,
        palette: super::preferences::Palette,
    ) -> El {
        use crate::ui_bridge::settings::Family;
        let page = &core.page;
        let connected = page.flow.borrow().is_connected();
        let busy = !connected || !page.controls_ready() || page.settings_blocked();
        let provider_refresh =
            page.controls_ready() && page.settings.borrow().can_refresh(Family::Providers);
        let agent_refresh =
            page.controls_ready() && page.settings.borrow().can_refresh(Family::Agents);
        let provider_uncertain = page.settings.borrow().uncertain(Family::Providers);
        let agent_uncertain = page.settings.borrow().uncertain(Family::Agents);
        let view = page.view.borrow();
        let mut out = col()
            .w_full()
            .gap(12.0)
            .color(palette.ink)
            .child(typography.text("Providers").role("heading"));
        out = out.child(button(
            typography,
            "Refresh providers",
            "Refresh providers".into(),
            "settings-provider-refresh".into(),
            provider_refresh,
            |state| Settings::refresh(state, Family::Providers),
        ));
        if provider_uncertain {
            out = out.child(
                typography
                    .text("The last provider change is uncertain. Refresh before trying again.")
                    .wrap(),
            );
        }
        if view.provider_settings.cleanup_pending {
            out = out.child(
                typography
                    .text("A saved provider entry is waiting to be removed.")
                    .wrap(),
            );
        }
        if let Some(notice) = &view.provider_settings.notice {
            out = out.child(typography.text(notice).wrap());
        }
        for (i, draft) in self.providers.iter().enumerate() {
            let row = view
                .provider_settings
                .connections
                .iter()
                .find(|row| row.protocol == draft.protocol);
            let conflict = row.is_some_and(|row| draft.dirty && row.revision != draft.revision);
            let enabled = !busy && row.is_some();
            let version = draft.version;
            let seen_revision = row.map(|row| row.revision);
            let label = match draft.protocol {
                ProviderProtocol::Messages => "Messages",
                ProviderProtocol::ChatCompletions => "Chat Completions",
                ProviderProtocol::Responses => "OpenAI Responses",
                ProviderProtocol::GoogleInteractions => "Google Interactions",
            };
            let mut section = col()
                .key(format!("settings-provider-{i}"))
                .w_full()
                .gap(6.0)
                .child(typography.text(label).role("heading"));
            if let Some(row) = row {
                let wrapper = UiProviderConnection::new(Rc::clone(page), row.clone());
                section = section.child(typography.text(wrapper.state()));
            }
            section = section
                .child(provider_input(
                    i,
                    ProviderField::Endpoint,
                    "Endpoint",
                    label,
                    &draft.endpoint,
                    enabled,
                    typography,
                    palette,
                ))
                .child(provider_input(
                    i,
                    ProviderField::Model,
                    "Model",
                    label,
                    &draft.model,
                    enabled,
                    typography,
                    palette,
                ));
            if draft.protocol == ProviderProtocol::ChatCompletions {
                let mut presets = ::rune::el::row().flex_wrap().gap(6.0);
                for (index, &(name, _, _)) in crate::model::CHAT_PRESETS.iter().enumerate() {
                    presets = presets.child(button(
                        typography,
                        name,
                        format!("Use the {name} endpoint for {label}"),
                        format!("settings-provider-{i}-preset-{index}"),
                        enabled,
                        move |state| Settings::chat_preset(state, i, version, index),
                    ));
                }
                section = section.child(presets);
                let keyless = draft.keyless;
                section = section
                    .child(button(
                        typography,
                        if keyless {
                            "No API key: on"
                        } else {
                            "No API key: off"
                        },
                        format!("{label} No API key: {}", if keyless { "on" } else { "off" }),
                        format!("settings-provider-{i}-keyless"),
                        enabled,
                        move |state| {
                            let Some(core) = state.core().cloned() else {
                                return;
                            };
                            if !core.page.controls_ready() || core.page.settings_blocked() {
                                return;
                            }
                            if let Some(draft) = state.settings.providers.get_mut(i)
                                && draft.version == version
                            {
                                draft.keyless = !draft.keyless;
                                draft.key.clear();
                                draft.dirty = true;
                                draft.version = draft.version.wrapping_add(1);
                            }
                        },
                    ))
                    .child(provider_input(
                        i,
                        ProviderField::Timeout,
                        "Response timeout (seconds)",
                        label,
                        &draft.timeout,
                        enabled,
                        typography,
                        palette,
                    ));
            }
            if !(draft.protocol == ProviderProtocol::ChatCompletions && draft.keyless) {
                section = section.child(provider_input(
                    i,
                    ProviderField::Key,
                    "API key",
                    label,
                    &draft.key,
                    enabled,
                    typography,
                    palette,
                ));
            }
            if conflict {
                let saved = row.unwrap();
                section = section
                    .child(
                        typography
                            .text(format!(
                                "Saved settings changed. Endpoint: {}. Model: {}.",
                                saved.endpoint.as_deref().unwrap_or("None"),
                                saved.model.as_deref().unwrap_or("None")
                            ))
                            .wrap(),
                    )
                    .child(button(
                        typography,
                        "Keep my edits",
                        format!("Keep my edits for {label} provider connection"),
                        format!("settings-provider-{i}-keep"),
                        enabled,
                        move |state| {
                            let Some(core) = state.core().cloned() else {
                                return;
                            };
                            if !core.page.controls_ready() || core.page.settings_blocked() {
                                return;
                            }
                            let Some(draft) = state.settings.providers.get_mut(i) else {
                                return;
                            };
                            if draft.version != version {
                                return;
                            }
                            let revision = core
                                .page
                                .view
                                .borrow()
                                .provider_settings
                                .connections
                                .iter()
                                .find(|row| row.protocol == draft.protocol)
                                .map(|row| row.revision);
                            if revision == seen_revision
                                && let Some(revision) = revision
                            {
                                draft.revision = revision;
                                draft.version = draft.version.wrapping_add(1);
                            }
                        },
                    ));
            }
            section = section
                .child(button(
                    typography,
                    "Save",
                    format!("Save {label} provider connection"),
                    format!("settings-provider-{i}-save"),
                    enabled && !conflict,
                    move |state| Settings::provider_action(state, i, version, true),
                ))
                .child(button(
                    typography,
                    "Remove",
                    format!("Remove {label} provider connection"),
                    format!("settings-provider-{i}-remove"),
                    enabled && !conflict,
                    move |state| Settings::provider_action(state, i, version, false),
                ))
                .child(button(
                    typography,
                    "Use saved values",
                    format!("Use saved values for {label} provider connection"),
                    format!("settings-provider-{i}-reset"),
                    enabled,
                    move |state| {
                        let Some(core) = state.core().cloned() else {
                            return;
                        };
                        if !core.page.controls_ready() || core.page.settings_blocked() {
                            return;
                        }
                        if let Some(draft) = state.settings.providers.get_mut(i) {
                            if draft.version != version
                                || !core
                                    .page
                                    .view
                                    .borrow()
                                    .provider_settings
                                    .connections
                                    .iter()
                                    .any(|row| {
                                        row.protocol == draft.protocol
                                            && Some(row.revision) == seen_revision
                                    })
                            {
                                return;
                            }
                            draft.key.clear();
                            draft.dirty = false;
                            draft.version = draft.version.wrapping_add(1);
                        }
                        state.settings.sync(&core);
                    },
                ));
            out = out.child(section);
        }
        out = out
            .child(typography.text("Installed agents").role("heading"))
            .child(button(
                typography,
                "Refresh agents",
                "Refresh agents".into(),
                "settings-agent-refresh".into(),
                agent_refresh,
                |state| Settings::refresh(state, Family::Agents),
            ));
        if agent_uncertain {
            out = out.child(
                typography
                    .text("The last agent change is uncertain. Refresh before trying again.")
                    .wrap(),
            );
        }
        for (i, draft) in self.agents.iter().enumerate() {
            let row = view
                .agent_settings
                .agents
                .iter()
                .find(|row| row.agent == draft.agent);
            let conflict = row.is_some_and(|row| draft.dirty && row.revision != draft.revision);
            let enabled = !busy && row.is_some();
            let version = draft.version;
            let seen_revision = row.map(|row| row.revision);
            let label = match draft.agent {
                AgentKind::Codex => "Codex",
                AgentKind::Opencode => "OpenCode",
                AgentKind::Pi => "Pi",
            };
            let mut section = col()
                .key(format!("settings-agent-{i}"))
                .w_full()
                .gap(6.0)
                .child(typography.text(label).role("heading"));
            if let Some(row) = row {
                let wrapper = UiAgentConfiguration::new(Rc::clone(page), row.clone());
                section = section.child(typography.text(wrapper.state()));
            }
            section = section
                .child(agent_input(
                    i,
                    AgentField::Executable,
                    "Executable",
                    label,
                    &draft.executable,
                    enabled,
                    typography,
                    palette,
                ))
                .child(agent_input(
                    i,
                    AgentField::Workspace,
                    "Workspace",
                    label,
                    &draft.workspace,
                    enabled,
                    typography,
                    palette,
                ));
            if draft.agent == AgentKind::Pi {
                section = section
                    .child(agent_input(
                        i,
                        AgentField::Session,
                        "Session directory",
                        label,
                        &draft.session_dir,
                        enabled,
                        typography,
                        palette,
                    ))
                    .child(agent_input(
                        i,
                        AgentField::Provider,
                        "Provider",
                        label,
                        &draft.provider,
                        enabled,
                        typography,
                        palette,
                    ))
                    .child(agent_input(
                        i,
                        AgentField::Model,
                        "Model",
                        label,
                        &draft.model,
                        enabled,
                        typography,
                        palette,
                    ))
                    .child(button(
                        typography,
                        if draft.offline {
                            "Offline: on"
                        } else {
                            "Offline: off"
                        },
                        format!(
                            "{label} Offline: {}",
                            if draft.offline { "on" } else { "off" }
                        ),
                        format!("settings-agent-{i}-offline"),
                        enabled,
                        move |state| {
                            let Some(core) = state.core().cloned() else {
                                return;
                            };
                            if !core.page.controls_ready() || core.page.settings_blocked() {
                                return;
                            }
                            if let Some(draft) = state.settings.agents.get_mut(i) {
                                if draft.version != version {
                                    return;
                                }
                                draft.offline = !draft.offline;
                                draft.dirty = true;
                                draft.version = draft.version.wrapping_add(1);
                            }
                        },
                    ));
            }
            if conflict {
                section = section
                    .child(
                        typography
                            .text("Saved settings changed. Review them before keeping your edits.")
                            .wrap(),
                    )
                    .child(
                        typography
                            .text(saved_profile(row.and_then(|row| row.profile.as_ref())))
                            .wrap(),
                    )
                    .child(button(
                        typography,
                        "Keep my edits",
                        format!("Keep my edits for {label} profile"),
                        format!("settings-agent-{i}-keep"),
                        enabled,
                        move |state| {
                            let Some(core) = state.core().cloned() else {
                                return;
                            };
                            if !core.page.controls_ready() || core.page.settings_blocked() {
                                return;
                            }
                            let Some(draft) = state.settings.agents.get_mut(i) else {
                                return;
                            };
                            if draft.version != version {
                                return;
                            }
                            let revision = core
                                .page
                                .view
                                .borrow()
                                .agent_settings
                                .agents
                                .iter()
                                .find(|row| row.agent == draft.agent)
                                .map(|row| row.revision);
                            if revision == seen_revision
                                && let Some(revision) = revision
                            {
                                draft.revision = revision;
                                draft.version = draft.version.wrapping_add(1);
                            }
                        },
                    ));
            }
            section = section
                .child(button(
                    typography,
                    "Save",
                    format!("Save {label} profile"),
                    format!("settings-agent-{i}-save"),
                    enabled && !conflict,
                    move |state| Settings::agent_action(state, i, version, true),
                ))
                .child(button(
                    typography,
                    "Remove",
                    format!("Remove {label} profile"),
                    format!("settings-agent-{i}-remove"),
                    enabled && !conflict,
                    move |state| Settings::agent_action(state, i, version, false),
                ))
                .child(button(
                    typography,
                    "Use saved values",
                    format!("Use saved values for {label} profile"),
                    format!("settings-agent-{i}-reset"),
                    enabled,
                    move |state| {
                        let Some(core) = state.core().cloned() else {
                            return;
                        };
                        if !core.page.controls_ready() || core.page.settings_blocked() {
                            return;
                        }
                        if let Some(draft) = state.settings.agents.get_mut(i) {
                            if draft.version != version
                                || !core.page.view.borrow().agent_settings.agents.iter().any(
                                    |row| {
                                        row.agent == draft.agent
                                            && Some(row.revision) == seen_revision
                                    },
                                )
                            {
                                return;
                            }
                            draft.dirty = false;
                            draft.version = draft.version.wrapping_add(1);
                        }
                        state.settings.sync(&core);
                    },
                ));
            out = out.child(section);
        }
        if !connected {
            out = out.child(
                typography
                    .text("Refresh to reconnect, or reload the panel.")
                    .wrap(),
            );
        }
        if let Some(message) = self.message {
            out = out.child(typography.text(message).wrap());
        }
        out
    }
}

use super::theme::ThemeCaret;
use super::*;
use crate::model::{AgentKind, AgentProfile, Protocol};
use std::{
    cell::RefCell,
    future::Future,
    pin::Pin,
    rc::{Rc, Weak},
};

type WorkFuture = Pin<Box<dyn Future<Output = Result<js_sys::Undefined, FlowError>>>>;

struct ProviderDraft {
    protocol: Protocol,
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
    /// The provider whose endpoint preset list is open.
    preset_open: Option<usize>,
}

const PRESET_TRIGGER: &str = "settings-preset-trigger";

/// The Chat Completions preset whose endpoint `draft` holds now.
fn chosen_preset(draft: &ProviderDraft) -> Option<usize> {
    metldr_core::gateway::chat_routes().position(|route| route.endpoint() == draft.endpoint.text)
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            owner: Weak::new(),
            flow: None,
            providers: Protocol::KEYED
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
            preset_open: None,
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
                // A decision connection not yet set up starts at its service and model.
                let decision = metldr_core::gateway::decision_route(draft.protocol)
                    .filter(|_| row.endpoint.is_none());
                let endpoint = decision.map(|route| route.endpoint());
                let model = decision.map(|_| metldr_core::gateway::DECISION_MODEL);
                let endpoint = row.endpoint.as_deref().or(endpoint.as_deref()).unwrap_or("");
                replace_if_changed(&mut draft.endpoint, endpoint);
                replace_if_changed(&mut draft.model, row.model.as_deref().or(model).unwrap_or(""));
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
        let chat = draft.protocol == Protocol::ChatCompletions;
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
        let (Some(draft), Some(route)) = (
            state.settings.providers.get_mut(index),
            metldr_core::gateway::chat_routes().nth(preset),
        ) else {
            return;
        };
        if draft.version != expected_version {
            return;
        }
        replace_if_changed(&mut draft.endpoint, &route.endpoint());
        draft.keyless = route.keyless();
        draft.key.clear();
        draft.dirty = true;
        draft.version = draft.version.wrapping_add(1);
    }

    /// Fills the executable with the one the companion found; the user still saves.
    fn use_found(state: &mut State, index: usize, expected_version: u64, path: &str) {
        let Some(core) = state.core().cloned() else {
            return;
        };
        if !core.page.controls_ready() || core.page.settings_blocked() {
            return;
        }
        let Some(draft) = state.settings.agents.get_mut(index) else {
            return;
        };
        if draft.version != expected_version {
            return;
        }
        replace_if_changed(&mut draft.executable, path);
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
                }
                | AgentProfile::Claude {
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
            AgentKind::Claude => AgentProfile::Claude {
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
            }
            | AgentProfile::Claude {
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

#[allow(clippy::too_many_arguments)]
fn provider_input(
    index: usize,
    field: ProviderField,
    label: &'static str,
    context: &str,
    edit: &TextEdit,
    enabled: bool,
    typography: super::preferences::Typography,
    palette: &super::preferences::Palette,
) -> El {
    let ink = palette.ink;
    let input = |input: El| match field {
        ProviderField::Key => input.secure().placeholder("Leave blank to keep the saved key", ink.alpha(0.55)),
        ProviderField::Timeout => input.placeholder("45", ink.alpha(0.55)),
        _ => input,
    };
    text_field(
        format!("settings-provider-{index}-{label}"),
        label,
        context,
        edit,
        enabled,
        typography,
        palette,
        move |state: &mut State| match field {
            ProviderField::Endpoint => &mut state.settings.providers[index].endpoint,
            ProviderField::Model => &mut state.settings.providers[index].model,
            ProviderField::Key => &mut state.settings.providers[index].key,
            ProviderField::Timeout => &mut state.settings.providers[index].timeout,
        },
        move |state: &mut State| {
            let draft = &mut state.settings.providers[index];
            draft.dirty = true;
            draft.version = draft.version.wrapping_add(1);
        },
        input,
    )
}

#[allow(clippy::too_many_arguments)]
fn agent_input(
    index: usize,
    field: AgentField,
    label: &'static str,
    context: &str,
    edit: &TextEdit,
    enabled: bool,
    typography: super::preferences::Typography,
    palette: &super::preferences::Palette,
) -> El {
    text_field(
        format!("settings-agent-{index}-{label}"),
        label,
        context,
        edit,
        enabled,
        typography,
        palette,
        move |state: &mut State| match field {
            AgentField::Executable => &mut state.settings.agents[index].executable,
            AgentField::Workspace => &mut state.settings.agents[index].workspace,
            AgentField::Session => &mut state.settings.agents[index].session_dir,
            AgentField::Provider => &mut state.settings.agents[index].provider,
            AgentField::Model => &mut state.settings.agents[index].model,
        },
        move |state: &mut State| {
            let draft = &mut state.settings.agents[index];
            draft.dirty = true;
            draft.version = draft.version.wrapping_add(1);
        },
        |input| input,
    )
}

/// One settings text field: its label over a single line input named "`context` `label`" that
/// edits through `edit_of`, marks its draft changed through `changed`, and takes `extra` (a
/// secret, a placeholder).
#[allow(clippy::too_many_arguments)]
fn text_field(
    key: String,
    label: &'static str,
    context: &str,
    edit: &TextEdit,
    enabled: bool,
    typography: super::preferences::Typography,
    palette: &super::preferences::Palette,
    edit_of: impl Fn(&mut State) -> &mut TextEdit + 'static,
    changed: impl Fn(&mut State) + 'static,
    extra: impl FnOnce(El) -> El,
) -> El {
    let ink = palette.ink;
    let input = ::rune::el::text_input(edit, typography.input())
        .key(key)
        .w_full()
        .min_h(36.0)
        .padding(6.0)
        .single_line()
        .fill(palette.field)
        .border(1.0, palette.line)
        .color(ink)
        .caret(ink, palette.selection)
        .theme_caret(palette)
        .disabled(!enabled)
        .on_edit(edit_of)
        .on_input_change(move |state: &mut State, _| changed(state));
    metldr_ui::field::field(
        typography.text(label),
        extra(input),
        format!("{context} {label}"),
        Default::default(),
        |line| typography.text(line.to_owned()),
    )
}

/// A settings action: an outlined key on the theme's material, read by `accessible_name`, that
/// sounds `label`'s cue and runs `action` when pressed or activated from the keyboard.
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
    let el = super::preferences::outlined(typography, label, key).border(1.0, typography.line).label(accessible_name);
    let props = metldr_ui::button::Props { disabled: !enabled, ..Default::default() };
    let el = metldr_ui::button::button(el, props, move |state: &mut State, _| {
        state.sound(cue);
        action(state)
    });
    if !enabled {
        el.opacity(0.5)
    } else if typography.lip.is_some() {
        el.press(typography.press)
    } else {
        el.press(typography.press).press_scale(super::theme::PRESS_SCALE)
    }
}

impl Settings {
    /// The select that fills provider `i`'s endpoint from a known provider.
    fn preset_trigger(&self, i: usize, label: &str, enabled: bool, typography: super::preferences::Typography, palette: &super::preferences::Palette) -> El {
        let draft = &self.providers[i];
        let (version, chosen) = (draft.version, chosen_preset(draft));
        let preset = chosen.and_then(|n| metldr_core::gateway::chat_routes().nth(n));
        let shown = preset.map_or("Custom endpoint", |route| route.name);
        let trigger = row()
            .key(PRESET_TRIGGER)
            .gap(6.0)
            .center_y()
            .child(typography.legend(shown))
            .child(::rune::el::icon(&super::composer::CHEVRON, 11.0).color(palette.ink_2))
            .label(format!("{label} endpoint preset, {shown}"));
        let trigger = super::preferences::keyed(typography, trigger).border(1.0, typography.line);
        if !enabled {
            return trigger.role("combobox").disabled(true).opacity(0.5);
        }
        let items: Rc<[metldr_ui::select::Item]> = metldr_core::gateway::chat_routes()
            .enumerate()
            .map(|(n, route)| metldr_ui::select::Item {
                key: format!("settings-preset-{n}"),
                label: route.name.into(),
                disabled: false,
            })
            .collect();
        let open = self.preset_open == Some(i);
        let trigger = metldr_ui::select::trigger(
            trigger,
            "settings-preset-list",
            open,
            items,
            chosen,
            move |state: &mut State, open, _| {
                state.sound(crate::interaction_audio::Cue::Press);
                state.settings.preset_open = open.then_some(i);
            },
            move |state: &mut State, n, _| Settings::chat_preset(state, i, version, n),
        );
        if typography.lip.is_some() { trigger.press(typography.press) } else { trigger.press(typography.press).press_scale(super::theme::PRESS_SCALE) }
    }

    /// The open endpoint preset list, drawn over the settings so their scroll never clips it.
    pub(super) fn preset_list(&self, typography: super::preferences::Typography, p: &super::preferences::Palette) -> Option<El> {
        let i = self.preset_open?;
        let draft = self.providers.get(i)?;
        let (version, chosen) = (draft.version, chosen_preset(draft));
        let close = |state: &mut State, _: &mut Tx| state.settings.preset_open = None;
        let options = metldr_core::gateway::chat_routes().enumerate().map(|(n, route)| {
            let name = route.name;
            let row = super::composer::menu_row(typography, p, format!("settings-preset-{n}"), name.into(), "", chosen == Some(n)).label(name).press(p.line_strong);
            metldr_ui::select::option(row, false, chosen == Some(n), move |state: &mut State, _| {
                state.sound(crate::interaction_audio::Cue::Press);
                Settings::chat_preset(state, i, version, n);
            }, close)
        });
        let list = col()
            .key("settings-preset-list")
            .min_w(176.0)
            .padding(4.0)
            .fill(p.surface)
            .border(1.0, p.line)
            .radius(10.0)
            .shadow(0.0, 8.0, p.line.alpha(0.28))
            .children(options);
        Some(metldr_ui::select::popup(list, "Endpoint presets", PRESET_TRIGGER, super::theme::pop_in(p), close))
    }

    /// Display inside the caller's settings route, after `sync`.
    pub(super) fn view(
        &self,
        core: &Rc<Core>,
        _cx: &mut Cx<State>,
        typography: super::preferences::Typography,
        palette: &super::preferences::Palette,
        open: (bool, bool),
    ) -> (Option<El>, Option<El>) {
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
        let providers = if open.0 {
            let mut out = col().w_full().gap(12.0).color(palette.ink);
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
                let label = draft.protocol.label();
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
                if draft.protocol == Protocol::ChatCompletions {
                    section = section.child(self.preset_trigger(i, label, enabled, typography, palette));
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
                if !(draft.protocol == Protocol::ChatCompletions && draft.keyless) {
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
            Some(out)
        } else {
            None
        };
        let agents = if open.1 {
            let mut out = col()
                .w_full()
                .gap(12.0)
                .color(palette.ink)
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
                let label = draft.agent.label();
                let mut section = col()
                    .key(format!("settings-agent-{i}"))
                    .w_full()
                    .gap(6.0)
                    .child(typography.text(label).role("heading"));
                if let Some(row) = row {
                    let wrapper = UiAgentConfiguration::new(Rc::clone(page), row.clone());
                    section = section.child(typography.text(wrapper.state()));
                    match &row.found {
                        Some(path) => {
                            section = section.child(typography.text(format!("Found at {path}")).wrap());
                            if draft.executable.text != *path {
                                let path = path.clone();
                                section = section.child(button(
                                    typography,
                                    "Use found",
                                    format!("Use the {label} found at {path}"),
                                    format!("settings-agent-{i}-found"),
                                    enabled,
                                    move |state| Settings::use_found(state, i, version, &path),
                                ));
                            }
                        }
                        None => section = section.child(typography.text("Not found on this computer").wrap()),
                    }
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
            if !view.agent_settings.apps.is_empty() {
                let mut apps = col()
                    .key("settings-desktop-apps")
                    .w_full()
                    .gap(6.0)
                    .child(typography.text("Desktop apps").role("heading"));
                for app in &view.agent_settings.apps {
                    let status = match (&app.config, app.local_config) {
                        (Some(path), _) => format!("{}: config at {path}", app.label),
                        (None, true) => format!("{}: not found", app.label),
                        (None, false) => format!("{}: add MetLDR in its own settings", app.label),
                    };
                    apps = apps.child(typography.text(status).wrap());
                }
                out = out.child(apps);
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
            Some(out)
        } else {
            None
        };
        (providers, agents)
    }
}

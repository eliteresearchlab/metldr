use metldr_core::gateway::{self, Route};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    Chrome,
    Local(Server),
    Provider,
    Agent,
}

/// A model server on this computer that the extension's gateway reaches on loopback.
pub type Server = &'static Route;

/// A service the user brings a key for; the gateway checks and stores the key.
pub type Service = &'static Route;

/// The local server with route `id`.
pub fn local(id: &str) -> Option<Server> {
    gateway::local_routes().find(|route| route.id == id)
}

/// The key service with route `id`.
pub fn keyed(id: &str) -> Option<Service> {
    gateway::key_routes().find(|route| route.id == id)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    Ready,
    Downloadable,
    Downloading,
    Unavailable,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Panel {
    Preparing,
    Ready(i32),
    Opening,
    Unavailable,
}

pub struct Setup {
    pub preview: bool,
    pub panel: Panel,
    pub choice: Choice,
    pub status: Status,
    pub generation: u64,
    pub models: Vec<String>,
    pub model: String,
    pub progress: Option<f64>,
    pub detail: String,
    pub loading: bool,
    pub saving: bool,
    pub theme: String,
    pub theme_saving: bool,
    /// The easiest working choice found on this computer, marked for the user.
    pub recommended: Option<Choice>,
    /// The local servers that answered, with their models.
    pub servers: Vec<(Server, Vec<String>)>,
    pub service: Service,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            preview: false,
            panel: Panel::Preparing,
            choice: Choice::Chrome,
            status: Status::Idle,
            generation: 0,
            models: Vec::new(),
            model: String::new(),
            progress: None,
            detail: "Checking what works on this computer. No prompt or page content is sent."
                .into(),
            loading: true,
            saving: false,
            theme: "catppuccin".into(),
            theme_saving: false,
            recommended: None,
            servers: Vec::new(),
            service: keyed("openai").expect("the route table lists openai"),
        }
    }
}

impl Setup {
    /// The choice saved earlier: the chat's destination first, then the gateway's preferred route.
    pub fn restore_choice(&mut self, destination: Option<&str>, preferred: Option<&str>) {
        let choice = match destination {
            Some(
                "browser"
                | "chat"
                | "responses"
                | "google"
                | "page:messages"
                | "page:responses"
                | "page:google_interactions"
                | "page:chat_completions",
            ) => Choice::Provider,
            Some("codex" | "opencode" | "pi") => Choice::Agent,
            Some(value)
                if ["codex:", "opencode:", "pi:"]
                    .into_iter()
                    .any(|prefix| value.strip_prefix(prefix).is_some_and(|id| !id.is_empty())) =>
            {
                Choice::Agent
            }
            Some(_) => {
                self.choose(Choice::Provider);
                self.detail =
                    "Review your saved destination in the panel before connecting.".into();
                return;
            }
            None => match (
                preferred.and_then(local),
                preferred.and_then(keyed),
            ) {
                (Some(server), _) => Choice::Local(server),
                (None, Some(service)) => {
                    self.service = service;
                    Choice::Provider
                }
                (None, None) => Choice::Chrome,
            },
        };
        self.choose(choice);
    }

    pub fn choose(&mut self, choice: Choice) {
        self.generation += 1;
        self.choice = choice;
        self.status = Status::Idle;
        self.progress = None;
        self.models.clear();
        self.detail = match choice {
            Choice::Chrome => "Check whether Chrome AI is ready on this device.".into(),
            Choice::Local(server) => match self.found(server) {
                Some(models) => {
                    self.models = models.to_vec();
                    self.status = if self.models.is_empty() {
                        Status::Unavailable
                    } else {
                        Status::Ready
                    };
                    if self.models.is_empty() {
                        format!("{} is running here but has no models yet. Add one, then check again.", server.name)
                    } else {
                        format!("{} is running here. Choose a model, then save.", server.name)
                    }
                }
                None => format!(
                    "Start {} at {}, then check again. The check lists models and sends no prompt.",
                    server.name,
                    server.address()
                ),
            },
            Choice::Provider => "Paste your key. metldr checks it with the service and keeps it encrypted in this browser.".into(),
            Choice::Agent => "In the panel, choose Settings, then Installed agents.".into(),
        };
        if !self.models.contains(&self.model) {
            self.model = self.models.first().cloned().unwrap_or_default();
        }
    }

    /// The models of `server` when it answered the last check.
    pub fn found(&self, server: Server) -> Option<&[String]> {
        self.servers
            .iter()
            .find(|(found, _)| *found == server)
            .map(|(_, models)| models.as_slice())
    }

    /// The easiest choice that works here: Chrome's built-in AI when Chrome offers it, then the
    /// first local server with a model, then the user's own key.
    pub fn recommend(chrome: Option<&str>, servers: &[(Server, Vec<String>)]) -> Choice {
        match chrome {
            Some("available" | "downloadable" | "downloading") => Choice::Chrome,
            _ => servers
                .iter()
                .find(|(_, models)| !models.is_empty())
                .map_or(Choice::Provider, |(server, _)| Choice::Local(server)),
        }
    }

    pub fn begin(&mut self, status: Status) -> u64 {
        self.generation += 1;
        self.status = status;
        self.progress = None;
        self.generation
    }

    pub fn current(&self, generation: u64) -> bool {
        self.generation == generation
    }

    pub fn ready(&self) -> bool {
        !self.preview
            && self.status == Status::Ready
            && match self.choice {
                Choice::Chrome => true,
                Choice::Local(_) | Choice::Provider => self.models.contains(&self.model),
                Choice::Agent => false,
            }
    }

    pub fn availability(&mut self, value: &str) {
        let (status, detail) = match value {
            "missing-api" => (
                Status::Unavailable,
                "Chrome does not expose the Summarizer API on this page. Check browser and device support in Help, or use what you have.",
            ),
            "available" => (Status::Ready, "Chrome AI is ready on this device."),
            "downloadable" => (
                Status::Downloadable,
                "Chrome can download the model. Download size depends on the browser and model version.",
            ),
            "downloading" => (
                Status::Downloading,
                "Chrome reports a download in progress. Check again later for its status.",
            ),
            "unavailable" => (
                Status::Unavailable,
                "Chrome AI is unavailable on this device. Use what you have, bring a key, or continue without connecting.",
            ),
            _ => (
                Status::Error,
                "Chrome returned an unknown availability state. Check again or choose another connection.",
            ),
        };
        self.status = status;
        self.detail = detail.into();
    }

    pub fn update_progress(&mut self, loaded: f64) {
        if loaded.is_finite() && (0.0..=1.0).contains(&loaded) {
            self.progress = Some(self.progress.unwrap_or(0.0).max(loaded));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(id: &str) -> Server {
        local(id).unwrap()
    }

    #[test]
    fn choices_invalidate_checks_and_require_a_current_model() {
        let mut setup = Setup::default();
        let first = setup.begin(Status::Checking);
        setup.servers = vec![(at("lmstudio"), vec!["qwen3-4b".into()])];
        setup.choose(Choice::Local(at("lmstudio")));
        assert!(!setup.current(first));
        assert_eq!(setup.status, Status::Ready);
        assert_eq!(setup.model, "qwen3-4b");
        assert!(setup.ready());
        setup.model = "deleted:latest".into();
        assert!(!setup.ready());
        setup.preview = true;
        setup.model = "qwen3-4b".into();
        assert!(!setup.ready());
        setup.preview = false;
        setup.choose(Choice::Local(at("llamacpp")));
        assert_eq!(setup.status, Status::Idle);
        assert!(setup.detail.contains("127.0.0.1:8080"));
        assert!(!setup.ready());
        setup.choose(Choice::Provider);
        setup.panel = Panel::Ready(7);
        setup.status = Status::Ready;
        assert!(!setup.ready());
        setup.models = vec!["gpt-5-mini".into()];
        setup.model = "gpt-5-mini".into();
        assert!(setup.ready());
        setup.choose(Choice::Agent);
        assert!(!setup.ready());
        assert_eq!(setup.panel, Panel::Ready(7));
    }

    #[test]
    fn availability_and_fractional_progress_are_not_estimates() {
        let mut setup = Setup::default();
        setup.choose(Choice::Chrome);
        setup.availability("downloadable");
        assert_eq!(setup.status, Status::Downloadable);
        assert_eq!(setup.progress, None);
        setup.availability("downloading");
        assert_eq!(setup.status, Status::Downloading);
        setup.update_progress(0.25);
        setup.update_progress(0.1);
        setup.update_progress(f64::NAN);
        setup.update_progress(50.0);
        assert_eq!(setup.progress, Some(0.25));
        setup.availability("available");
        assert!(setup.ready());
        setup.availability("unexpected");
        assert_eq!(setup.status, Status::Error);
    }

    #[test]
    fn saved_destinations_take_precedence_over_the_preferred_route() {
        assert_eq!(Setup::default().choice, Choice::Chrome);
        for (destination, preferred, expected) in [
            (None, None, Choice::Chrome),
            (None, Some("chrome-ai"), Choice::Chrome),
            (None, Some("ollama"), Choice::Local(at("ollama"))),
            (None, Some("lmstudio"), Choice::Local(at("lmstudio"))),
            (None, Some("llamacpp"), Choice::Local(at("llamacpp"))),
            (None, Some("anthropic"), Choice::Provider),
            (Some("browser"), Some("ollama"), Choice::Provider),
            (Some("chat"), None, Choice::Provider),
            (Some("page:chat_completions"), None, Choice::Provider),
            (Some("codex"), None, Choice::Agent),
            (Some("pi:missing"), None, Choice::Agent),
            (Some("codex:missing"), Some("ollama"), Choice::Agent),
            (Some("unknown"), Some("ollama"), Choice::Provider),
            (Some("codex:"), Some("chrome-ai"), Choice::Provider),
        ] {
            let mut setup = Setup::default();
            setup.restore_choice(destination, preferred);
            assert_eq!(setup.choice, expected, "{destination:?} / {preferred:?}");
            assert!(!setup.ready());
        }
        let mut setup = Setup::default();
        setup.restore_choice(None, Some("anthropic"));
        assert_eq!(setup.service.id, "anthropic");
        setup.restore_choice(Some("unknown"), Some("ollama"));
        assert!(setup.detail.contains("Review your saved destination"));
    }

    #[test]
    fn the_easiest_working_choice_is_recommended() {
        let found = vec![
            (at("ollama"), Vec::new()),
            (at("llamacpp"), vec!["gemma-3-1b".to_string()]),
        ];
        assert_eq!(Setup::recommend(Some("available"), &found), Choice::Chrome);
        assert_eq!(Setup::recommend(Some("downloadable"), &[]), Choice::Chrome);
        assert_eq!(
            Setup::recommend(Some("unavailable"), &found),
            Choice::Local(at("llamacpp"))
        );
        assert_eq!(Setup::recommend(None, &found[..1]), Choice::Provider);
        assert_eq!(Setup::recommend(Some("missing-api"), &[]), Choice::Provider);
    }
}

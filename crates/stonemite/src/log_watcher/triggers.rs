use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use regex::Regex;

use eqlog::{LogEventDomain, LogSource, LogSourceId, ParsedLogEvent, RawLogLine};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct TriggerDefinition {
    pub id: Arc<str>,
    pub matcher: TriggerMatcher,
    pub scope: TriggerScope,
    pub presentation: Vec<PresentationAction>,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum TriggerMatcher {
    RawText {
        text: Arc<str>,
        case_sensitive: bool,
    },
    RawRegex(Arc<str>),
    EventDomain(LogEventDomain),
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerScope {
    SourceClient { source_id: LogSourceId },
    AllClients,
    Global,
}

/// The trigger boundary can produce presentation and telemetry effects only.
/// There is intentionally no arbitrary callback, command, keyboard, mouse, or
/// Trushar-control variant in this type.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresentationAction {
    ShowText { text: Arc<str> },
    FlashBorder,
    PlaySound { path: PathBuf },
    Speak { text: Arc<str> },
    StartTimer(TimerRequest),
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimerRequest {
    pub id: Arc<str>,
    pub label: Arc<str>,
    pub duration: Duration,
}

#[cfg(debug_assertions)]
pub(super) const QA_TIMER_PHRASE: &str = "stonemite timer";

#[cfg(debug_assertions)]
pub(super) fn qa_timer_definition() -> TriggerDefinition {
    TriggerDefinition {
        id: Arc::from("qa-timer-trigger"),
        // Nearby clients also log normal /say text. Restrict the QA hook to
        // the originating client's local echo so only that box starts.
        matcher: TriggerMatcher::RawRegex(Arc::from(format!(
            r"(?i)^You say, .*{}",
            regex::escape(QA_TIMER_PHRASE)
        ))),
        scope: TriggerScope::AllClients,
        presentation: vec![PresentationAction::StartTimer(TimerRequest {
            id: Arc::from("qa-timer"),
            label: Arc::from("QA timer"),
            duration: Duration::from_secs(10),
        })],
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerActivation {
    pub trigger_id: Arc<str>,
    pub scope: TriggerScope,
    pub source: LogSource,
    pub presentation: Arc<[PresentationAction]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerDefinitionError {
    pub trigger_id: Arc<str>,
    pub message: String,
}

struct CompiledTrigger {
    id: Arc<str>,
    matcher: CompiledMatcher,
    scope: TriggerScope,
    presentation: Arc<[PresentationAction]>,
}

enum CompiledMatcher {
    Text {
        text: Arc<str>,
        lowercase_text: Option<String>,
    },
    Regex(Regex),
    EventDomain(LogEventDomain),
}

pub(crate) struct TriggerEngine {
    triggers: Vec<CompiledTrigger>,
}

impl TriggerEngine {
    pub fn new() -> Self {
        Self {
            triggers: Vec::new(),
        }
    }

    #[allow(dead_code)]
    pub fn replace_definitions(
        &mut self,
        definitions: Vec<TriggerDefinition>,
    ) -> Vec<TriggerDefinitionError> {
        let mut compiled = Vec::new();
        let mut errors = Vec::new();
        for definition in definitions {
            match compile(definition) {
                Ok(trigger) => compiled.push(trigger),
                Err(error) => errors.push(error),
            }
        }
        self.triggers = compiled;
        errors
    }

    pub fn evaluate(&self, line: &RawLogLine, events: &[ParsedLogEvent]) -> Vec<TriggerActivation> {
        let lowercase_body = self
            .triggers
            .iter()
            .any(|trigger| {
                matches!(
                    trigger.matcher,
                    CompiledMatcher::Text {
                        lowercase_text: Some(_),
                        ..
                    }
                )
            })
            .then(|| line.body.to_ascii_lowercase());

        self.triggers
            .iter()
            .filter(|trigger| scope_matches(&trigger.scope, &line.source))
            .filter(|trigger| match &trigger.matcher {
                CompiledMatcher::Text {
                    text,
                    lowercase_text: None,
                } => line.body.contains(text.as_ref()),
                CompiledMatcher::Text {
                    lowercase_text: Some(text),
                    ..
                } => lowercase_body
                    .as_ref()
                    .is_some_and(|body| body.contains(text)),
                CompiledMatcher::Regex(regex) => regex.is_match(&line.body),
                CompiledMatcher::EventDomain(domain) => {
                    events.iter().any(|event| event.event.domain() == *domain)
                }
            })
            .map(|trigger| TriggerActivation {
                trigger_id: trigger.id.clone(),
                scope: trigger.scope.clone(),
                source: line.source.clone(),
                presentation: trigger.presentation.clone(),
            })
            .collect()
    }
}

fn compile(definition: TriggerDefinition) -> Result<CompiledTrigger, TriggerDefinitionError> {
    let matcher = match definition.matcher {
        TriggerMatcher::RawText {
            text,
            case_sensitive,
        } => CompiledMatcher::Text {
            lowercase_text: (!case_sensitive).then(|| text.to_ascii_lowercase()),
            text,
        },
        TriggerMatcher::RawRegex(pattern) => Regex::new(&pattern)
            .map(CompiledMatcher::Regex)
            .map_err(|error| TriggerDefinitionError {
                trigger_id: definition.id.clone(),
                message: error.to_string(),
            })?,
        TriggerMatcher::EventDomain(domain) => CompiledMatcher::EventDomain(domain),
    };
    Ok(CompiledTrigger {
        id: definition.id,
        matcher,
        scope: definition.scope,
        presentation: definition.presentation.into(),
    })
}

fn scope_matches(scope: &TriggerScope, source: &LogSource) -> bool {
    match scope {
        TriggerScope::SourceClient { source_id } => source_id == &source.id,
        TriggerScope::AllClients | TriggerScope::Global => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(source_id: &str, body: &str) -> RawLogLine {
        RawLogLine {
            source: LogSource::new(source_id, "Bilka", "teek"),
            timestamp: None,
            body: Arc::from(body),
        }
    }

    #[test]
    fn compiles_regex_once_and_rejects_invalid_definitions() {
        let mut engine = TriggerEngine::new();
        let errors = engine.replace_definitions(vec![TriggerDefinition {
            id: Arc::from("bad"),
            matcher: TriggerMatcher::RawRegex(Arc::from("(")),
            scope: TriggerScope::Global,
            presentation: vec![PresentationAction::FlashBorder],
        }]);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].trigger_id.as_ref(), "bad");
        assert!(engine
            .evaluate(&line("client-1", "anything"), &[])
            .is_empty());
    }

    #[test]
    fn source_scoping_and_presentation_only_actions_are_enforced_by_types() {
        let mut engine = TriggerEngine::new();
        assert!(engine
            .replace_definitions(vec![TriggerDefinition {
                id: Arc::from("tell"),
                matcher: TriggerMatcher::RawText {
                    text: Arc::from("tells you"),
                    case_sensitive: false,
                },
                scope: TriggerScope::SourceClient {
                    source_id: LogSourceId::new("client-7"),
                },
                presentation: vec![
                    PresentationAction::ShowText {
                        text: Arc::from("Incoming tell"),
                    },
                    PresentationAction::StartTimer(TimerRequest {
                        id: Arc::from("tell-visible"),
                        label: Arc::from("Tell"),
                        duration: Duration::from_secs(5),
                    }),
                ],
            }])
            .is_empty());

        assert!(engine
            .evaluate(&line("client-6", "Bob tells you, hello"), &[])
            .is_empty());
        let activations = engine.evaluate(&line("client-7", "Bob TELLS YOU, hello"), &[]);
        assert_eq!(activations.len(), 1);
        assert_eq!(activations[0].presentation.len(), 2);
        assert!(matches!(
            &activations[0].presentation[1],
            PresentationAction::StartTimer(request)
                if request.id.as_ref() == "tell-visible"
                    && request.label.as_ref() == "Tell"
                    && request.duration == Duration::from_secs(5)
        ));
    }
}

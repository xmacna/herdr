// Modified under the Apache License, Version 2.0; see LICENSE.
use serde::{de::Deserializer, Deserialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrganizationLocale {
    Portuguese,
    English,
}

impl Default for OrganizationLocale {
    fn default() -> Self {
        Self::Portuguese
    }
}

impl OrganizationLocale {
    pub fn parse(value: &str) -> Self {
        if value.trim().eq_ignore_ascii_case("en") || value.trim().eq_ignore_ascii_case("english") {
            Self::English
        } else {
            Self::Portuguese
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Portuguese => "pt",
            Self::English => "en",
        }
    }
}

/// Client-local organization metadata for presenting agents as a hierarchy.
/// Runtime state remains owned by the endpoint and is matched by pane_id.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct OrganizationConfig {
    pub enabled: bool,
    pub registry_path: Option<String>,
    /// Client-local language for the organization surface. Portuguese is the
    /// compatibility default; invalid or missing values fall back to it.
    pub locale: String,
    pub root: String,
    pub reserve_label: String,
    pub orphan_label: String,
    pub pending_label: String,
    pub agents: Vec<OrganizationAgentConfig>,
}

impl OrganizationConfig {
    pub fn locale(&self) -> OrganizationLocale {
        OrganizationLocale::parse(&self.locale)
    }

    pub fn root_label(&self) -> &str {
        self.root_label_for(OrganizationLocale::Portuguese)
    }

    pub fn root_label_for(&self, locale: OrganizationLocale) -> &str {
        if self.root.trim().is_empty() {
            match locale {
                OrganizationLocale::Portuguese => "Organização",
                OrganizationLocale::English => "Organization",
            }
        } else {
            self.root.trim()
        }
    }

    pub fn reserve_label(&self) -> &str {
        self.reserve_label_for(OrganizationLocale::Portuguese)
    }

    pub fn reserve_label_for(&self, locale: OrganizationLocale) -> &str {
        if self.reserve_label.trim().is_empty() {
            match locale {
                OrganizationLocale::Portuguese => "Reserva",
                OrganizationLocale::English => "Reserve",
            }
        } else {
            self.reserve_label.trim()
        }
    }

    pub fn orphan_label(&self) -> &str {
        self.orphan_label_for(OrganizationLocale::Portuguese)
    }

    pub fn orphan_label_for(&self, locale: OrganizationLocale) -> &str {
        if self.orphan_label.trim().is_empty() {
            match locale {
                OrganizationLocale::Portuguese => "Sem cadastro",
                OrganizationLocale::English => "Unassigned",
            }
        } else {
            self.orphan_label.trim()
        }
    }

    pub fn pending_label(&self) -> &str {
        self.pending_label_for(OrganizationLocale::Portuguese)
    }

    pub fn pending_label_for(&self, locale: OrganizationLocale) -> &str {
        if self.pending_label.trim().is_empty() {
            match locale {
                OrganizationLocale::Portuguese => "Cadastro pendente",
                OrganizationLocale::English => "Registration pending",
            }
        } else {
            self.pending_label.trim()
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct OrganizationAgentConfig {
    /// Stable local registration key. It is never inferred from display text.
    pub id: String,
    pub label: String,
    /// Optional human-facing name. The runtime's technical `name` is kept
    /// separately and is never promoted to presentation text.
    pub display_name: Option<String>,
    pub name: Option<String>,
    /// Existing client-visible runtime target. Pane ids survive shell reconnects.
    #[serde(alias = "runtime_id")]
    pub pane_id: Option<String>,
    #[serde(alias = "machine", alias = "endpoint")]
    pub endpoint_id: Option<String>,
    #[serde(alias = "parent")]
    pub parent_id: Option<String>,
    pub role: String,
    #[serde(alias = "owner")]
    pub owner_id: Option<String>,
    pub area: Option<String>,
    #[serde(deserialize_with = "deserialize_task")]
    pub task: Option<String>,
    pub function: Option<String>,
    pub model: Option<String>,
    pub reserved: bool,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OrganizationTaskValue {
    Summary(String),
    Detailed { summary: Option<String> },
}

fn deserialize_task<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<OrganizationTaskValue>::deserialize(deserializer)?;
    Ok(match value {
        Some(OrganizationTaskValue::Summary(summary)) => Some(summary),
        Some(OrganizationTaskValue::Detailed { summary, .. }) => summary,
        None => None,
    })
}

#[cfg(test)]
mod tests {
    use super::{OrganizationConfig, OrganizationLocale};

    #[test]
    fn registrations_survive_config_toml_restore_without_runtime_tokens() {
        let config: crate::config::Config = toml::from_str(
            r#"
                [ui.organization]
                enabled = true
                root = "Operations"
                reserve_label = "Bench"

                [[ui.organization.agents]]
                id = "luna"
                label = "Luna"
                pane_id = "machine-a:p1"
                endpoint_id = "ssh:machine-a"
                role = "worker"
                area = "release"
                task = "verification"
                model = "gpt-5.6-luna"
            "#,
        )
        .expect("organization config should restore");

        let registration = &config.ui.organization.agents[0];
        assert!(config.ui.organization.enabled);
        assert_eq!(config.ui.organization.root_label(), "Operations");
        assert_eq!(registration.id, "luna");
        assert_eq!(registration.pane_id.as_deref(), Some("machine-a:p1"));
        assert_eq!(registration.endpoint_id.as_deref(), Some("ssh:machine-a"));
        assert_eq!(registration.model.as_deref(), Some("gpt-5.6-luna"));
    }

    #[test]
    fn organization_locale_accepts_english_and_defaults_to_portuguese() {
        let english: crate::config::Config = toml::from_str(
            r#"
                [ui.organization]
                enabled = true
                locale = "en"
            "#,
        )
        .expect("organization locale should restore");

        assert_eq!(
            english.ui.organization.locale(),
            OrganizationLocale::English
        );
        assert_eq!(
            OrganizationConfig::default().locale(),
            OrganizationLocale::Portuguese
        );
        assert_eq!(
            OrganizationConfig::default().orphan_label_for(OrganizationLocale::English),
            "Unassigned"
        );
        assert_eq!(OrganizationConfig::default().reserve_label(), "Reserva");
        assert_eq!(OrganizationConfig::default().orphan_label(), "Sem cadastro");
        assert_eq!(
            OrganizationConfig::default().pending_label(),
            "Cadastro pendente"
        );
    }
}

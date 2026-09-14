// Modified under the Apache License, Version 2.0; see LICENSE.
use serde::Deserialize;

/// Client-local organization metadata for presenting agents as a hierarchy.
/// Runtime state remains owned by the endpoint and is matched by pane_id.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct OrganizationConfig {
    pub enabled: bool,
    pub root: String,
    pub reserve_label: String,
    pub orphan_label: String,
    pub pending_label: String,
    pub agents: Vec<OrganizationAgentConfig>,
}

impl OrganizationConfig {
    pub fn root_label(&self) -> &str {
        if self.root.trim().is_empty() {
            "Organization"
        } else {
            self.root.trim()
        }
    }

    pub fn reserve_label(&self) -> &str {
        if self.reserve_label.trim().is_empty() {
            "Reserve"
        } else {
            self.reserve_label.trim()
        }
    }

    pub fn orphan_label(&self) -> &str {
        if self.orphan_label.trim().is_empty() {
            "Unassigned"
        } else {
            self.orphan_label.trim()
        }
    }

    pub fn pending_label(&self) -> &str {
        if self.pending_label.trim().is_empty() {
            "Pending"
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
    /// Stable live Herdr agent name. When present, it is the primary runtime identity.
    /// `pane_id` remains a fallback for older generated configs and offline entries.
    #[serde(alias = "agent_name")]
    pub name: Option<String>,
    pub label: String,
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
    pub task: Option<String>,
    pub model: Option<String>,
    pub reserved: bool,
}

#[cfg(test)]
mod tests {
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
                name = "wk-release-luna"
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
        assert_eq!(registration.name.as_deref(), Some("wk-release-luna"));
        assert_eq!(registration.pane_id.as_deref(), Some("machine-a:p1"));
        assert_eq!(registration.endpoint_id.as_deref(), Some("ssh:machine-a"));
        assert_eq!(registration.model.as_deref(), Some("gpt-5.6-luna"));
    }
}

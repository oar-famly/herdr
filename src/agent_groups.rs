use serde::{Deserialize, Serialize};

/// Sessions saved before custom groups existed marked agents as archived; those
/// agents are filed into a group with this id.
pub const LEGACY_ARCHIVED_GROUP_ID: &str = "archived";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentGroup {
    pub id: String,
    pub label: String,
}

/// User-defined, ordered sections of the agent panel. Agents outside every group
/// belong to the implicit default section, which always comes first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentGroups {
    #[serde(default)]
    pub groups: Vec<AgentGroup>,
    #[serde(default)]
    pub next_number: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentGroupError {
    EmptyLabel,
    LabelTaken(String),
    NotFound(String),
}

impl AgentGroupError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::EmptyLabel => "agent_group_label_empty",
            Self::LabelTaken(_) => "agent_group_label_taken",
            Self::NotFound(_) => "agent_group_not_found",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::EmptyLabel => "agent group label must not be empty".into(),
            Self::LabelTaken(label) => format!("an agent group named {label:?} already exists"),
            Self::NotFound(group) => format!("agent group {group} not found"),
        }
    }
}

impl AgentGroups {
    pub fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.groups.iter().any(|group| group.id == id)
    }

    /// Scripts name a group by id or by label; labels are unique ignoring case.
    pub fn resolve(&self, key: &str) -> Result<&AgentGroup, AgentGroupError> {
        let key = key.trim();
        self.groups
            .iter()
            .find(|group| group.id == key)
            .or_else(|| {
                self.groups
                    .iter()
                    .find(|group| group.label.eq_ignore_ascii_case(key))
            })
            .ok_or_else(|| AgentGroupError::NotFound(key.to_owned()))
    }

    pub fn create(&mut self, label: &str) -> Result<AgentGroup, AgentGroupError> {
        let label = self.available_label(label, None)?;
        let id = loop {
            self.next_number += 1;
            let id = format!("g{}", self.next_number);
            if !self.contains(&id) {
                break id;
            }
        };
        let group = AgentGroup { id, label };
        self.groups.push(group.clone());
        Ok(group)
    }

    pub fn rename(&mut self, key: &str, label: &str) -> Result<AgentGroup, AgentGroupError> {
        let id = self.resolve(key)?.id.clone();
        let label = self.available_label(label, Some(&id))?;
        let group = self
            .groups
            .iter_mut()
            .find(|group| group.id == id)
            .ok_or_else(|| AgentGroupError::NotFound(key.to_owned()))?;
        group.label = label;
        Ok(group.clone())
    }

    pub fn move_by(&mut self, key: &str, offset: isize) -> Result<AgentGroup, AgentGroupError> {
        let id = self.resolve(key)?.id.clone();
        let from = self
            .groups
            .iter()
            .position(|group| group.id == id)
            .ok_or_else(|| AgentGroupError::NotFound(key.to_owned()))?;
        let to = from
            .saturating_add_signed(offset)
            .min(self.groups.len().saturating_sub(1));
        let group = self.groups.remove(from);
        self.groups.insert(to, group.clone());
        Ok(group)
    }

    pub fn remove(&mut self, key: &str) -> Result<AgentGroup, AgentGroupError> {
        let id = self.resolve(key)?.id.clone();
        let index = self
            .groups
            .iter()
            .position(|group| group.id == id)
            .ok_or_else(|| AgentGroupError::NotFound(key.to_owned()))?;
        Ok(self.groups.remove(index))
    }

    /// Restores a group an agent still points at, or reports that the reference is stale.
    pub fn adopt(&mut self, id: &str) -> bool {
        if self.contains(id) {
            return true;
        }
        if id != LEGACY_ARCHIVED_GROUP_ID {
            return false;
        }
        let label = (1..)
            .map(|n| {
                if n == 1 {
                    LEGACY_ARCHIVED_GROUP_ID.to_owned()
                } else {
                    format!("{LEGACY_ARCHIVED_GROUP_ID} {n}")
                }
            })
            .find(|label| {
                !self
                    .groups
                    .iter()
                    .any(|group| group.label.eq_ignore_ascii_case(label))
            })
            .unwrap_or_default();
        self.groups.push(AgentGroup {
            id: id.to_owned(),
            label,
        });
        true
    }

    fn available_label(
        &self,
        label: &str,
        except_id: Option<&str>,
    ) -> Result<String, AgentGroupError> {
        let label = label.trim();
        if label.is_empty() {
            return Err(AgentGroupError::EmptyLabel);
        }
        if self.groups.iter().any(|group| {
            Some(group.id.as_str()) != except_id && group.label.eq_ignore_ascii_case(label)
        }) {
            return Err(AgentGroupError::LabelTaken(label.to_owned()));
        }
        Ok(label.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(groups: &AgentGroups) -> Vec<&str> {
        groups
            .groups
            .iter()
            .map(|group| group.label.as_str())
            .collect()
    }

    #[test]
    fn groups_keep_creation_order_and_resolve_by_id_or_label() {
        let mut groups = AgentGroups::default();
        let reviews = groups.create(" Reviews ").unwrap();
        groups.create("parked").unwrap();
        assert_eq!(labels(&groups), ["Reviews", "parked"]);
        assert_eq!(groups.resolve(&reviews.id).unwrap().label, "Reviews");
        assert_eq!(groups.resolve("reviews").unwrap().id, reviews.id);
        assert_eq!(
            groups.resolve("missing").unwrap_err(),
            AgentGroupError::NotFound("missing".into())
        );
    }

    #[test]
    fn labels_must_be_present_and_unique_ignoring_case() {
        let mut groups = AgentGroups::default();
        let parked = groups.create("parked").unwrap();
        assert_eq!(
            groups.create("  ").unwrap_err(),
            AgentGroupError::EmptyLabel
        );
        assert_eq!(
            groups.create("PARKED").unwrap_err(),
            AgentGroupError::LabelTaken("PARKED".into())
        );
        assert_eq!(groups.rename(&parked.id, "Parked").unwrap().label, "Parked");
    }

    #[test]
    fn deleted_group_ids_are_never_reused() {
        let mut groups = AgentGroups::default();
        let first = groups.create("a").unwrap();
        groups.remove(&first.id).unwrap();
        assert_ne!(groups.create("b").unwrap().id, first.id);
    }

    #[test]
    fn moving_a_group_stops_at_either_end() {
        let mut groups = AgentGroups::default();
        for label in ["a", "b", "c"] {
            groups.create(label).unwrap();
        }
        groups.move_by("c", -1).unwrap();
        assert_eq!(labels(&groups), ["a", "c", "b"]);
        groups.move_by("a", -1).unwrap();
        groups.move_by("b", 1).unwrap();
        assert_eq!(labels(&groups), ["a", "c", "b"]);
    }

    #[test]
    fn only_the_legacy_archive_reference_is_adopted() {
        let mut groups = AgentGroups::default();
        groups.create("archived").unwrap();
        assert!(!groups.adopt("g42"));
        assert!(groups.adopt(LEGACY_ARCHIVED_GROUP_ID));
        assert!(groups.adopt(LEGACY_ARCHIVED_GROUP_ID));
        assert_eq!(labels(&groups), ["archived", "archived 2"]);
    }
}

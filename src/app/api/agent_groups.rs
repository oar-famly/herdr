use crate::agent_groups::AgentGroupError;
use crate::api::schema::{
    AgentGroupCreateParams, AgentGroupInfo, AgentGroupMoveDirection, AgentGroupMoveParams,
    AgentGroupRenameParams, AgentGroupTarget, PaneGroupSetParams, ResponseResult,
};
use crate::app::App;

use super::responses::{encode_error, encode_success};

fn group_error(id: String, error: AgentGroupError) -> String {
    encode_error(id, error.code(), error.message())
}

fn group_result(group: &crate::agent_groups::AgentGroup) -> ResponseResult {
    ResponseResult::AgentGroup {
        group: AgentGroupInfo::from(group),
    }
}

impl App {
    pub(super) fn handle_agent_group_list(&self, id: String) -> String {
        encode_success(
            id,
            ResponseResult::AgentGroupList {
                groups: self
                    .state
                    .agent_groups
                    .groups
                    .iter()
                    .map(AgentGroupInfo::from)
                    .collect(),
            },
        )
    }

    pub(super) fn handle_agent_group_create(
        &mut self,
        id: String,
        params: AgentGroupCreateParams,
    ) -> String {
        match self.state.agent_groups.create(&params.label) {
            Ok(group) => {
                self.state.mark_session_dirty();
                encode_success(id, group_result(&group))
            }
            Err(error) => group_error(id, error),
        }
    }

    pub(super) fn handle_agent_group_rename(
        &mut self,
        id: String,
        params: AgentGroupRenameParams,
    ) -> String {
        match self.state.agent_groups.rename(&params.group, &params.label) {
            Ok(group) => {
                self.state.mark_session_dirty();
                encode_success(id, group_result(&group))
            }
            Err(error) => group_error(id, error),
        }
    }

    pub(super) fn handle_agent_group_move(
        &mut self,
        id: String,
        params: AgentGroupMoveParams,
    ) -> String {
        let offset = match params.direction {
            AgentGroupMoveDirection::Up => -1,
            AgentGroupMoveDirection::Down => 1,
        };
        match self.state.agent_groups.move_by(&params.group, offset) {
            Ok(group) => {
                self.state.mark_session_dirty();
                encode_success(id, group_result(&group))
            }
            Err(error) => group_error(id, error),
        }
    }

    pub(super) fn handle_agent_group_delete(
        &mut self,
        id: String,
        params: AgentGroupTarget,
    ) -> String {
        let group = match self.state.agent_groups.remove(&params.group) {
            Ok(group) => group,
            Err(error) => return group_error(id, error),
        };
        let mut released = Vec::new();
        for (ws_idx, workspace) in self.state.workspaces.iter().enumerate() {
            for tab in &workspace.tabs {
                for pane_id in tab.panes.keys() {
                    if let Some(terminal_id) = tab.terminal_id(*pane_id) {
                        released.push((ws_idx, *pane_id, terminal_id.clone()));
                    }
                }
            }
        }
        released.retain(|(_, _, terminal_id)| {
            self.state
                .terminals
                .get_mut(terminal_id)
                .filter(|terminal| terminal.agent_group.as_deref() == Some(group.id.as_str()))
                .is_some_and(|terminal| terminal.set_agent_group(None))
        });
        self.state.mark_session_dirty();
        for (ws_idx, pane_id, _) in released {
            self.emit_pane_updated(ws_idx, pane_id);
        }
        encode_success(id, group_result(&group))
    }

    pub(super) fn handle_pane_group_set(
        &mut self,
        id: String,
        params: PaneGroupSetParams,
    ) -> String {
        let Some((ws_idx, pane_id)) = self.parse_pane_id(&params.pane_id) else {
            return encode_error(
                id,
                "pane_not_found",
                format!("pane {} not found", params.pane_id),
            );
        };
        let group = match params.group.as_deref() {
            Some(key) => match self.state.agent_groups.resolve(key) {
                Ok(group) => Some(group.id.clone()),
                Err(error) => return group_error(id, error),
            },
            None => None,
        };
        let Some(terminal) = self
            .state
            .workspaces
            .get(ws_idx)
            .and_then(|ws| ws.terminal_id(pane_id))
            .and_then(|terminal_id| self.state.terminals.get_mut(terminal_id))
        else {
            return encode_error(
                id,
                "pane_not_found",
                format!("pane {} not found", params.pane_id),
            );
        };
        if terminal.set_agent_group(group) {
            self.state.mark_session_dirty();
            self.emit_pane_updated(ws_idx, pane_id);
        }
        match self.pane_info(ws_idx, pane_id) {
            Some(pane) => encode_success(id, ResponseResult::PaneInfo { pane }),
            None => encode_error(
                id,
                "pane_not_found",
                format!("pane {} not found", params.pane_id),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{EventKind, SuccessResponse};
    use crate::workspace::Workspace;

    fn app() -> (App, String) {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![Workspace::test_new("groups")];
        app.state.ensure_test_terminals();
        let pane_id = app.state.workspaces[0].tabs[0].root_pane;
        let public_pane_id = app.public_pane_id(0, pane_id).unwrap();
        (app, public_pane_id)
    }

    fn success(response: String) -> ResponseResult {
        serde_json::from_str::<SuccessResponse>(&response)
            .unwrap_or_else(|_| panic!("expected success: {response}"))
            .result
    }

    fn created(app: &mut App, label: &str) -> AgentGroupInfo {
        let ResponseResult::AgentGroup { group } = success(app.handle_agent_group_create(
            "req".into(),
            AgentGroupCreateParams {
                label: label.into(),
            },
        )) else {
            panic!("create should return the group");
        };
        group
    }

    fn pane_group(app: &App, public_pane_id: &str) -> Option<String> {
        let (ws_idx, pane_id) = app.parse_pane_id(public_pane_id).unwrap();
        app.pane_info(ws_idx, pane_id).unwrap().agent_group
    }

    #[test]
    fn panes_join_groups_by_label_without_being_closed() {
        let (mut app, pane) = app();
        let reviews = created(&mut app, "reviews");
        app.state.session_dirty = false;
        let first_event = app.event_hub.current_sequence();

        let ResponseResult::PaneInfo { pane: info } = success(app.handle_pane_group_set(
            "req".into(),
            PaneGroupSetParams {
                pane_id: pane.clone(),
                group: Some("Reviews".into()),
            },
        )) else {
            panic!("pane.group.set should return pane info");
        };
        assert_eq!(info.agent_group.as_deref(), Some(reviews.group_id.as_str()));
        assert!(app.state.session_dirty);
        assert!(app
            .event_hub
            .events_after(first_event)
            .iter()
            .any(|(_, event)| event.event == EventKind::PaneUpdated));
        assert_eq!(app.state.workspaces[0].tabs[0].panes.len(), 1);

        success(app.handle_pane_group_set(
            "req".into(),
            PaneGroupSetParams {
                pane_id: pane.clone(),
                group: None,
            },
        ));
        assert_eq!(pane_group(&app, &pane), None);
    }

    #[test]
    fn deleting_a_group_returns_its_agents_to_the_default_section() {
        let (mut app, pane) = app();
        let parked = created(&mut app, "parked");
        success(app.handle_pane_group_set(
            "req".into(),
            PaneGroupSetParams {
                pane_id: pane.clone(),
                group: Some(parked.group_id.clone()),
            },
        ));

        success(app.handle_agent_group_delete(
            "req".into(),
            AgentGroupTarget {
                group: "parked".into(),
            },
        ));

        assert_eq!(pane_group(&app, &pane), None);
        assert!(app.state.agent_groups.is_empty());
        assert_eq!(app.state.workspaces[0].tabs[0].panes.len(), 1);
    }

    #[test]
    fn group_errors_use_stable_codes() {
        let (mut app, pane) = app();
        created(&mut app, "reviews");
        for (response, code) in [
            (
                app.handle_agent_group_create(
                    "req".into(),
                    AgentGroupCreateParams {
                        label: "REVIEWS".into(),
                    },
                ),
                "agent_group_label_taken",
            ),
            (
                app.handle_agent_group_rename(
                    "req".into(),
                    AgentGroupRenameParams {
                        group: "reviews".into(),
                        label: " ".into(),
                    },
                ),
                "agent_group_label_empty",
            ),
            (
                app.handle_pane_group_set(
                    "req".into(),
                    PaneGroupSetParams {
                        pane_id: pane,
                        group: Some("missing".into()),
                    },
                ),
                "agent_group_not_found",
            ),
            (
                app.handle_pane_group_set(
                    "req".into(),
                    PaneGroupSetParams {
                        pane_id: "missing:p9".into(),
                        group: None,
                    },
                ),
                "pane_not_found",
            ),
        ] {
            assert!(response.contains(code), "{response}");
        }
    }

    #[test]
    fn groups_list_in_their_moved_order() {
        let (mut app, _) = app();
        created(&mut app, "a");
        created(&mut app, "b");
        success(app.handle_agent_group_move(
            "req".into(),
            AgentGroupMoveParams {
                group: "b".into(),
                direction: AgentGroupMoveDirection::Up,
            },
        ));
        let ResponseResult::AgentGroupList { groups } =
            success(app.handle_agent_group_list("req".into()))
        else {
            panic!("list should return groups");
        };
        assert_eq!(
            groups
                .iter()
                .map(|group| group.label.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
    }

    #[test]
    fn restored_legacy_archive_becomes_a_group_and_stale_groups_are_dropped() {
        let (mut app, pane) = app();
        let (ws_idx, pane_id) = app.parse_pane_id(&pane).unwrap();
        let terminal_id = app.state.workspaces[ws_idx]
            .terminal_id(pane_id)
            .unwrap()
            .clone();
        app.state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .agent_group = Some(crate::agent_groups::LEGACY_ARCHIVED_GROUP_ID.into());
        app.state.reconcile_agent_groups();
        assert_eq!(
            app.state.agent_groups.groups[0].label,
            crate::agent_groups::LEGACY_ARCHIVED_GROUP_ID
        );

        app.state
            .terminals
            .get_mut(&terminal_id)
            .unwrap()
            .agent_group = Some("g99".into());
        app.state.reconcile_agent_groups();
        assert_eq!(app.state.terminals[&terminal_id].agent_group, None);
    }
}

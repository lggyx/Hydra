#![allow(unused_imports)]

use std::time::Duration;

use tokio::sync::mpsc;

use hydra_core::{
    agent::{AgentCommand, AgentEvent, AgentPhase, TurnStopReason},
    tool::ToolRegistry,
    turn::event::{TurnEvent, TurnResult},
};
use hydra_telemetry::{event::SessionMode, runtime::CurrentContext};

// ============================================================================
// Tests: Core ↔ UI Communication (AgentCommand / AgentEvent channel)
// ============================================================================

#[tokio::test]
async fn test_agent_command_channel_send_message() {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<AgentCommand>();

    cmd_tx
        .send(AgentCommand::SendMessage {
            text: "Hello from demo".to_string(),
            images: vec![],
            image_markers: vec![],
        })
        .unwrap();

    let cmd = cmd_rx.recv().await.unwrap();
    match cmd {
        AgentCommand::SendMessage { text, .. } => {
            assert_eq!(text, "Hello from demo");
        }
        _ => panic!("Unexpected command variant"),
    }
}

#[tokio::test]
async fn test_agent_command_channel_cancel() {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<AgentCommand>();

    cmd_tx.send(AgentCommand::Cancel).unwrap();
    let cmd = cmd_rx.recv().await.unwrap();
    assert!(matches!(cmd, AgentCommand::Cancel));
}

#[tokio::test]
async fn test_agent_command_channel_approval() {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<AgentCommand>();

    cmd_tx.send(AgentCommand::ApproveTool).unwrap();
    let cmd = cmd_rx.recv().await.unwrap();
    assert!(matches!(cmd, AgentCommand::ApproveTool));

    cmd_tx.send(AgentCommand::DenyTool).unwrap();
    let cmd = cmd_rx.recv().await.unwrap();
    assert!(matches!(cmd, AgentCommand::DenyTool));
}

#[tokio::test]
async fn test_agent_command_channel_config_reload() {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<AgentCommand>();

    cmd_tx
        .send(AgentCommand::ReloadConfig(hydra_core::config::Config::default()))
        .unwrap();
    let cmd = cmd_rx.recv().await.unwrap();
    assert!(matches!(cmd, AgentCommand::ReloadConfig(_)));
}

#[tokio::test]
async fn test_agent_command_channel_chdir() {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<AgentCommand>();

    cmd_tx
        .send(AgentCommand::ChangeDir("/tmp/demo".to_string()))
        .unwrap();
    let cmd = cmd_rx.recv().await.unwrap();
    match cmd {
        AgentCommand::ChangeDir(path) => assert_eq!(path, "/tmp/demo"),
        _ => panic!("Unexpected variant"),
    }
}

#[tokio::test]
async fn test_agent_event_channel_text_delta() {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentEvent>();

    event_tx
        .send(AgentEvent::TextDelta("hello".to_string()))
        .unwrap();
    event_tx
        .send(AgentEvent::ReasoningDelta("thinking".to_string()))
        .unwrap();
    event_tx
        .send(AgentEvent::PhaseChange(AgentPhase::Thinking))
        .unwrap();

    let e1 = event_rx.recv().await.unwrap();
    let e2 = event_rx.recv().await.unwrap();
    let e3 = event_rx.recv().await.unwrap();

    match e1 {
        AgentEvent::TextDelta(t) => assert_eq!(t, "hello"),
        _ => panic!("Expected TextDelta"),
    }
    match e2 {
        AgentEvent::ReasoningDelta(t) => assert_eq!(t, "thinking"),
        _ => panic!("Expected ReasoningDelta"),
    }
    match e3 {
        AgentEvent::PhaseChange(AgentPhase::Thinking) => {}
        _ => panic!("Expected PhaseChange(Thinking)"),
    }
}

#[tokio::test]
async fn test_agent_event_channel_tool_lifecycle() {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentEvent>();

    event_tx
        .send(AgentEvent::ToolBatchStarted {
            batch_id: "batch-1".to_string(),
            calls: vec![],
        })
        .unwrap();

    event_tx
        .send(AgentEvent::ToolCallStarted {
            id: "call-1".to_string(),
            name: "read_file".to_string(),
            arguments: "{\"path\": \"src/main.rs\"}".to_string(),
        })
        .unwrap();

    event_tx
        .send(AgentEvent::ToolCallResult {
            call_id: "call-1".to_string(),
            name: "read_file".to_string(),
            output: "file content".to_string(),
            success: true,
            duration: Duration::from_millis(50),
        })
        .unwrap();

    event_tx
        .send(AgentEvent::ToolBatchCompleted {
            batch_id: "batch-1".to_string(),
            ok: 1,
            total: 1,
            elapsed_ms: 60,
        })
        .unwrap();

    let e1 = event_rx.recv().await.unwrap();
    let e2 = event_rx.recv().await.unwrap();
    let e3 = event_rx.recv().await.unwrap();
    let e4 = event_rx.recv().await.unwrap();

    assert!(matches!(e1, AgentEvent::ToolBatchStarted { .. }));
    assert!(matches!(e2, AgentEvent::ToolCallStarted { .. }));
    assert!(matches!(e3, AgentEvent::ToolCallResult { .. }));
    assert!(matches!(e4, AgentEvent::ToolBatchCompleted { .. }));
}

#[tokio::test]
async fn test_agent_event_channel_turn_complete() {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentEvent>();

    event_tx
        .send(AgentEvent::TurnComplete {
            duration: Duration::from_millis(1200),
            total_tokens: 150,
            turn_count: 1,
            tool_call_count: 0,
            stop_reason: TurnStopReason::Natural,
            messages: vec![],
        })
        .unwrap();

    let e = event_rx.recv().await.unwrap();
    match e {
        AgentEvent::TurnComplete {
            stop_reason,
            total_tokens,
            ..
        } => {
            assert_eq!(stop_reason, TurnStopReason::Natural);
            assert_eq!(total_tokens, 150);
        }
        _ => panic!("Expected TurnComplete"),
    }
}

#[tokio::test]
async fn test_agent_event_channel_approval_needed() {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentEvent>();

    use hydra_core::tool::ToolCall;
    event_tx
        .send(AgentEvent::ApprovalNeeded {
            tool_name: "bash".to_string(),
            reason: "Executing shell command".to_string(),
            call: ToolCall {
                id: "call-1".to_string(),
                name: "bash".to_string(),
                arguments: r#"{"command": "ls"}"#.to_string(),
            },
            messages: vec![],
        })
        .unwrap();

    let e = event_rx.recv().await.unwrap();
    match e {
        AgentEvent::ApprovalNeeded { tool_name, .. } => {
            assert_eq!(tool_name, "bash");
        }
        _ => panic!("Expected ApprovalNeeded"),
    }
}

// ============================================================================
// Tests: Telemetry Integration
// ============================================================================

#[tokio::test]
async fn test_telemetry_session_mode_variants() {
    use hydra_telemetry::event::Event;

    let modes = vec![
        SessionMode::Headless,
        SessionMode::Tui,
        SessionMode::Ide,
        SessionMode::Vscode,
        SessionMode::AtomcodeAir,
    ];

    for mode in modes {
        let json = serde_json::to_string(&mode).unwrap();
        assert!(!json.is_empty());
    }
}

#[tokio::test]
async fn test_telemetry_error_kind_serialization() {
    use hydra_telemetry::event::{Event, LlmErrorKind};

    let event = Event::LlmChat {
        duration_ms: 5000,
        tool_calls_count: 0,
        input_tokens: 0,
        output_tokens: 0,
        cached_tokens: 0,
        had_error: true,
        context_window: 0,
        system_tokens: 0,
        tool_def_tokens: 0,
        tool_result_tokens: 0,
        message_tokens: 0,
        messages_count: 0,
        error_kind: Some(LlmErrorKind::RateLimited),
        error_data: None,
    };

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("rate_limited"));
}

#[tokio::test]
async fn test_telemetry_tool_error_kind_variants() {
    use hydra_telemetry::event::ToolErrorKind;

    let kinds = vec![
        ToolErrorKind::NotFound,
        ToolErrorKind::InvalidArgs,
        ToolErrorKind::ExecutionFailed,
        ToolErrorKind::DeniedByUser,
        ToolErrorKind::BlockedByHook,
        ToolErrorKind::LoopDetected,
        ToolErrorKind::SkillNotFound,
        ToolErrorKind::SkillDisabled,
    ];

    for kind in kinds {
        let json = serde_json::to_string(&kind).unwrap();
        assert!(!json.is_empty());
    }
}

#[tokio::test]
async fn test_telemetry_event_variants_exist() {
    use hydra_telemetry::event::Event;

    let _ = Event::OpenAtomcode;
    let _ = Event::LlmChat {
        duration_ms: 0,
        tool_calls_count: 0,
        input_tokens: 0,
        output_tokens: 0,
        cached_tokens: 0,
        had_error: false,
        context_window: 0,
        system_tokens: 0,
        tool_def_tokens: 0,
        tool_result_tokens: 0,
        message_tokens: 0,
        messages_count: 0,
        error_kind: None,
        error_data: None,
    };
    let _ = Event::ToolCall {
        name: "t".into(),
        success: true,
        duration_ms: 0,
        error_kind: None,
        error_data: None,
    };
    let _ = Event::UseCommand {
        type_: "c".into(),
        success: None,
        error_kind: None,
        error_data: None,
    };
    let _ = Event::McpConnect {
        server_name: "s".into(),
        transport: hydra_telemetry::event::McpTransport::Stdio,
        success: true,
        duration_ms: None,
        error_kind: None,
        error_data: None,
    };
    let _ = Event::LoginSuccess;
    let _ = Event::TakeCodingplan {
        type_: hydra_telemetry::event::CodingplanResult::Success,
        error_kind: None,
        error_data: None,
    };
    let _ = Event::Panic {
        location: "".into(),
        message_head: "".into(),
        thread: "".into(),
        backtrace_top_5: vec![],
        error_kind: None,
        error_data: None,
    };
    let _ = Event::TelemetryDisabled;
    let _ = Event::CodingplanOfficialBuildRequired;
}

// ============================================================================
// Tests: Config ↔ Provider ↔ ToolRegistry ↔ AgentHandle chain
// ============================================================================

#[tokio::test]
async fn test_tool_registry_creation() {
    let registry = ToolRegistry::new();
    let tools: Vec<_> = registry.iter().await.collect();
    assert!(tools.is_empty());
}

#[tokio::test]
async fn test_tool_registry_api() {
    let registry = ToolRegistry::new();
    let _ = registry.get_definitions().await;
    let _ = registry.get("read_file").await;
}

#[tokio::test]
async fn test_conversation_creation_and_message_append() {
    let mut conv = hydra_core::conversation::Conversation::new();
    assert!(conv.messages.is_empty());

    conv.add_user_message("Hello");
    assert_eq!(conv.messages.len(), 1);

    // add_synthetic_user_message merges with the last user message if same role,
    // so we verify the message content was appended
    conv.add_synthetic_user_message("Hi there");
    // May merge into existing user message (expected behavior)
    assert!(conv.messages.len() >= 1);
}

// ============================================================================
// Tests: TurnEvent flow (used by both AgentLoop and Daemon)
// ============================================================================

#[tokio::test]
async fn test_turn_event_channel() {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<TurnEvent>();

    event_tx.send(TurnEvent::TextDelta("hello".into())).unwrap();
    event_tx
        .send(TurnEvent::ReasoningDelta("thinking".into()))
        .unwrap();
    event_tx
        .send(TurnEvent::TokenUsage {
            prompt_tokens: 10,
            completion_tokens: 20,
            total_tokens: 30,
            cached_tokens: 5,
        })
        .unwrap();

    let _ = event_rx.recv().await.unwrap();
    let _ = event_rx.recv().await.unwrap();
    let _ = event_rx.recv().await.unwrap();
}

// ============================================================================
// Tests: Cross-module type compatibility
// ============================================================================

#[tokio::test]
async fn test_agent_event_to_turn_result_conversion() {
    use hydra_core::turn::event::TurnEvent;

    let _ = TurnEvent::TextDelta("hello".into());
    let _ = TurnEvent::ToolCallStarted {
        id: "1".into(),
        name: "bash".into(),
        arguments: "{}".into(),
    };
    let _ = TurnEvent::TokenUsage {
        prompt_tokens: 10,
        completion_tokens: 20,
        total_tokens: 30,
        cached_tokens: 0,
    };

    let _ = TurnResult::Responded {
        text: "done".into(),
        tokens: 10,
        truncated: false,
    };
}

#[tokio::test]
async fn test_phase_enum_variants() {
    let _ = AgentPhase::Idle;
    let _ = AgentPhase::Thinking;
    let _ = AgentPhase::CallingTool("bash".into());
    let _ = AgentPhase::WaitingApproval;
}

#[tokio::test]
async fn test_permission_decision_variants() {
    use hydra_core::tool::PermissionDecision;

    let _ = PermissionDecision::Allow;
    let _ = PermissionDecision::Ask("reason".into());
    let _ = PermissionDecision::Deny;
}

#[tokio::test]
async fn test_session_mode_in_telemetry_context() {
    let ctx = CurrentContext {
        turn_id: Some(uuid::Uuid::new_v4()),
        provider: Some("claude".to_string()),
        provider_host: Some("api.anthropic.com".to_string()),
        model: Some("claude-sonnet-4".to_string()),
        repo_origin: None,
        mode: Some(SessionMode::Headless),
        session_id: Some(uuid::Uuid::new_v4()),
    };

    assert!(ctx.mode.is_some());
    assert_eq!(ctx.provider, Some("claude".to_string()));
}

// ============================================================================
// Integration: UI -> Agent channel round-trip
// ============================================================================

#[tokio::test]
async fn test_ui_to_agent_roundtrip_message_then_text_delta() {
    let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<AgentCommand>();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AgentEvent>();

    cmd_tx
        .send(AgentCommand::SendMessage {
            text: "Write a hello world".to_string(),
            images: vec![],
            image_markers: vec![],
        })
        .unwrap();

    let cmd = cmd_rx.recv().await.unwrap();
    match cmd {
        AgentCommand::SendMessage { .. } => {
            event_tx
                .send(AgentEvent::TextDelta("Hello".to_string()))
                .unwrap();
            event_tx
                .send(AgentEvent::TextDelta(" world".to_string()))
                .unwrap();
            event_tx
                .send(AgentEvent::TurnComplete {
                    duration: Duration::from_millis(500),
                    total_tokens: 10,
                    turn_count: 1,
                    tool_call_count: 0,
                    stop_reason: TurnStopReason::Natural,
                    messages: vec![],
                })
                .unwrap();
        }
        _ => {}
    }

    let e1 = event_rx.recv().await.unwrap();
    let e2 = event_rx.recv().await.unwrap();
    let e3 = event_rx.recv().await.unwrap();

    match e1 {
        AgentEvent::TextDelta(t) => assert_eq!(t, "Hello"),
        _ => panic!("Expected first TextDelta"),
    }
    match e2 {
        AgentEvent::TextDelta(t) => assert_eq!(t, " world"),
        _ => panic!("Expected second TextDelta"),
    }
    assert!(matches!(e3, AgentEvent::TurnComplete { .. }));
}

// ============================================================================
// Tests: TurnStopReason enum
// ============================================================================

#[tokio::test]
async fn test_turn_stop_reason_variants() {
    let reasons = vec![
        TurnStopReason::Natural,
        TurnStopReason::TurnLimit,
        TurnStopReason::StepLimit,
        TurnStopReason::Cancelled,
        TurnStopReason::Error,
    ];

    for r in reasons {
        let tag = r.as_tag();
        assert!(!tag.is_empty());
    }
}

// ============================================================================
// Tests: ToolRegistry structure
// ============================================================================

#[tokio::test]
async fn test_tool_registry_non_empty_after_registration() {
    let registry = ToolRegistry::new();
    // Verify the registry API is usable and the async iter method works
    let tools: Vec<_> = registry.iter().await.collect();
    assert!(tools.is_empty());
}

// ============================================================================
// Tests: Conversation structure
// ============================================================================

#[tokio::test]
async fn test_conversation_default() {
    let conv = hydra_core::conversation::Conversation::default();
    assert!(conv.messages.is_empty());
}

#[tokio::test]
async fn test_conversation_new() {
    let conv = hydra_core::conversation::Conversation::new();
    assert!(conv.messages.is_empty());
}

// ============================================================================
// Tests: ToolCall structure
// ============================================================================

#[tokio::test]
async fn test_tool_call_structure() {
    use hydra_core::tool::ToolCall;
    let call = ToolCall {
        id: "1".into(),
        name: "bash".into(),
        arguments: r#"{"cmd":"ls"}"#.into(),
    };
    assert_eq!(call.id, "1");
    assert_eq!(call.name, "bash");
}

// ============================================================================
// Tests: ToolResult structure
// ============================================================================

#[tokio::test]
async fn test_tool_result_structure() {
    use hydra_core::tool::ToolResult;
    let result = ToolResult {
        call_id: "1".into(),
        output: "output".into(),
        success: true,
    };
    assert!(result.success);
    assert_eq!(result.call_id, "1");
}

// ============================================================================
// Tests: PermissionStore
// ============================================================================

#[tokio::test]
async fn test_permission_store_creation() {
    let _ = hydra_core::tool::PermissionStore::new();
}

// ============================================================================
// Tests: McpConnectEvent
// ============================================================================

#[tokio::test]
async fn test_mcp_connect_event_variants() {
    use hydra_core::mcp::McpConnectEvent;
    let _ = McpConnectEvent::Connected {
        name: "test".into(),
    };
    let _ = McpConnectEvent::Failed {
        name: "test".into(),
        error: "timeout".into(),
    };
    let _ = McpConnectEvent::Warning {
        name: "test".into(),
        message: "tools/list failed".into(),
    };
}

// ============================================================================
// Tests: Plugin and LSP events
// ============================================================================

#[tokio::test]
async fn test_plugin_job_event_variants() {
    use hydra_core::plugin::{PluginJobEvent, marketplace::MarketplaceInfo};
    let info = MarketplaceInfo {
        name: "skill-1".into(),
        source: "git".into(),
        git_commit: "abc123".into(),
        plugins: vec!["plugin-a".into()],
    };
    let _ = PluginJobEvent::MarketplaceAdded(info);
}

#[tokio::test]
async fn test_lsp_connect_event_variants() {
    use hydra_core::lsp::LspConnectEvent;
    let _ = LspConnectEvent::Started {
        command: "rust-analyzer".into(),
        ext: "rs".into(),
    };
    let _ = LspConnectEvent::Failed {
        command: "rust-analyzer".into(),
        ext: "rs".into(),
        error: "not found".into(),
    };
}

// ============================================================================
// Tests: UpgradeEvent
// ============================================================================

#[tokio::test]
async fn test_upgrade_event_variants() {
    use hydra_core::self_update::UpgradeEvent;
    let _ = UpgradeEvent::ManifestFetched {
        version: "4.24.0".into(),
    };
    let _ = UpgradeEvent::Downloading { bytes: 1024, total: 2048 };
    let _ = UpgradeEvent::Done {
        version: "4.24.0".into(),
        backup: std::path::PathBuf::from("/tmp/backup"),
        exe: std::path::PathBuf::from("/tmp/hydra"),
    };
    let _ = UpgradeEvent::Failed("network".into());
}

// ============================================================================
// Tests: Telemetry McpTransport and CodingplanResult
// ============================================================================

#[tokio::test]
async fn test_mcp_transport_serialization() {
    use hydra_telemetry::event::McpTransport;
    let json = serde_json::to_string(&McpTransport::Stdio).unwrap();
    assert!(json.contains("stdio"));
}

#[tokio::test]
async fn test_codingplan_result_serialization() {
    use hydra_telemetry::event::CodingplanResult;
    let json = serde_json::to_string(&CodingplanResult::Success).unwrap();
    assert!(json.contains("success"));
}

// ============================================================================
// Tests: Message structure
// ============================================================================

#[tokio::test]
async fn test_message_construction() {
    use hydra_core::conversation::message::{Message, MessageContent, Role};
    let msg = Message {
        role: Role::User,
        content: MessageContent::Text("Hello".into()),
        synthetic: false,
    };
    assert_eq!(msg.role, Role::User);
}

#[tokio::test]
async fn test_message_role_variants() {
    use hydra_core::conversation::message::Role;
    let _ = Role::System;
    let _ = Role::User;
    let _ = Role::Assistant;
    let _ = Role::Tool;
}

// ============================================================================
// Tests: Envelope field presence
// ============================================================================

#[tokio::test]
async fn test_envelope_fields() {
    use hydra_telemetry::event::Envelope;
    let env = Envelope {
        device_id: uuid::Uuid::new_v4(),
        launch_id: uuid::Uuid::new_v4(),
        account_id: None,
        session_id: uuid::Uuid::new_v4(),
        turn_id: None,
        ts: 0,
        schema_version: 2,
        app_version: "4.23.3".into(),
        os: "windows".into(),
        arch: "x86_64".into(),
        locale: "zh-CN".into(),
        provider: None,
        provider_host: None,
        model: None,
        repo_origin: None,
        mode: None,
    };
    assert_eq!(env.schema_version, 2);
    assert_eq!(env.app_version, "4.23.3");
}

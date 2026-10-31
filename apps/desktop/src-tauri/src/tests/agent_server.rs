use super::*;
use app_core::{
    data::DataStore,
    ports::{SecretError, SecretVault},
    school_auth::SchoolAuthService,
};

struct Vault;
impl SecretVault for Vault {
    fn load(&self, _: &str) -> Result<Option<String>, SecretError> {
        Ok(None)
    }
    fn store(&self, _: &str, _: &str) -> Result<(), SecretError> {
        Ok(())
    }
    fn delete(&self, _: &str) -> Result<(), SecretError> {
        Ok(())
    }
}
struct Fixture {
    root: PathBuf,
    core: Arc<Core>,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("isct-mcp-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = DataStore::initialize_dir(root.clone()).unwrap().0;
        let core = Arc::new(Core::new(
            store,
            Arc::new(SchoolAuthService::with_status_cache(
                Vault,
                root.join("auth.json"),
            )),
        ));
        Self { root, core }
    }
    fn enabled(&self, enabled: bool) {
        self.core
            .store
            .put_workspace_setting(
                "devicePreferences",
                json!({"agentAccess":{"enabled":enabled}}),
            )
            .unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn token_is_persistent_private_and_atomically_published() {
    let fixture = Fixture::new();
    let path = fixture.root.join("agent-token");
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || load_or_create_token(&path).unwrap())
        })
        .collect();
    let values: Vec<_> = threads
        .into_iter()
        .map(|task| task.join().unwrap())
        .collect();
    assert!(values.iter().all(|token| token == &values[0]));
    assert_eq!(load_or_create_token(&path).unwrap(), values[0]);
    #[cfg(unix)]
    {
        use std::os::unix::{fs::symlink, fs::PermissionsExt};
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let alias = fixture.root.join("alias-token");
        symlink(&path, &alias).unwrap();
        assert!(load_or_create_token(&alias).is_err());
    }
}

fn request(
    client: &reqwest::Client,
    connection: &ConnectionInfo,
    method: &str,
    params: Value,
) -> reqwest::RequestBuilder {
    client
        .post(&connection.url)
        .bearer_auth(&connection.token)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-11-25")
        .json(&json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params}))
}
async fn rpc(
    client: &reqwest::Client,
    connection: &ConnectionInfo,
    method: &str,
    params: Value,
) -> Value {
    let response = request(client, connection, method, params)
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success(), "HTTP {}", response.status());
    assert!(response.headers().get("mcp-session-id").is_none());
    response.json().await.unwrap()
}

#[tokio::test]
async fn http_protocol_permissions_mutations_and_shutdown() {
    let fixture = Fixture::new();
    let server = AgentServer::bind(fixture.core.clone(), fixture.root.join("agent-token"), 0)
        .await
        .unwrap();
    let connection = server.connection();
    let client = app_core::net::client_builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let init = rpc(&client, &connection, "initialize", json!({
        "protocolVersion":"2025-11-25", "capabilities":{}, "clientInfo":{"name":"acceptance", "version":"1"}
    })).await;
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert!(
        rpc(&client, &connection, "tools/list", json!({})).await["result"]["tools"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let call = |name: &str, args: Value| json!({"name":name,"arguments":args});
    let closed = rpc(
        &client,
        &connection,
        "tools/call",
        call("grades", json!({})),
    )
    .await;
    assert_eq!(closed["result"]["isError"], true);
    // A failure carries Core's stable code for the agent to act on.
    let failure: Value =
        serde_json::from_str(closed["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(failure["code"], "agent-disabled");
    fixture.enabled(true);
    let listed = rpc(&client, &connection, "tools/list", json!({})).await;
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), app_core::agent::tools().len());
    let listed_tool = |name: &str| {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap()
            .clone()
    };
    // The hints and schemas come from Core's contracts.
    let today_tool = listed_tool("today");
    assert_eq!(today_tool["annotations"]["readOnlyHint"], true);
    assert_eq!(today_tool["annotations"]["openWorldHint"], false);
    assert_eq!(today_tool["outputSchema"]["type"], "object");
    let delete_tool = listed_tool("weekly_event_delete");
    assert_eq!(delete_tool["annotations"]["readOnlyHint"], false);
    assert_eq!(delete_tool["annotations"]["destructiveHint"], true);
    assert_eq!(delete_tool["inputSchema"]["additionalProperties"], false);
    assert!(delete_tool.get("_meta").is_none());
    // An LMS write reaches the school and asks the person every time.
    let save_tool = listed_tool("submission_save");
    assert_eq!(save_tool["annotations"]["destructiveHint"], true);
    assert_eq!(save_tool["annotations"]["openWorldHint"], true);
    assert_eq!(
        save_tool["_meta"]["anthropic/requiresUserInteraction"],
        true
    );
    let grades = rpc(
        &client,
        &connection,
        "tools/call",
        call("grades", json!({})),
    )
    .await;
    assert_eq!(grades["result"]["structuredContent"], json!({"grades": []}));
    let today = rpc(&client, &connection, "tools/call", call("today", json!({}))).await;
    let mut expected = fixture.core.agent.call("today", json!({})).unwrap();
    let mut actual = today["result"]["structuredContent"].clone();
    // These are separate reads; the wall-clock timestamp naturally advances.
    let now = actual.as_object_mut().unwrap().remove("now").unwrap();
    chrono::DateTime::parse_from_rfc3339(now.as_str().unwrap()).unwrap();
    expected.as_object_mut().unwrap().remove("now");
    assert_eq!(actual, expected);
    let created = rpc(
        &client,
        &connection,
        "tools/call",
        call(
            "weekly_event_create",
            json!({
                "term":"2026-3Q", "title":"HTTP fixture", "days":[1],
                "startTime":"09:00", "endTime":"09:45"
            }),
        ),
    )
    .await;
    assert!(created["result"]["structuredContent"]["id"].is_string());
    let schedule = rpc(
        &client,
        &connection,
        "tools/call",
        call("schedule", json!({"from":"2026-10-19","to":"2026-10-19"})),
    )
    .await;
    assert_eq!(
        schedule["result"]["structuredContent"]["events"][0]["title"],
        "HTTP fixture"
    );
    assert_eq!(
        fixture.core.store.load_workspace().unwrap()["settings"]["agentAccess"]["lastWrites"][0]
            ["tool"],
        "weekly_event_create"
    );
    let denied = rpc(
        &client,
        &connection,
        "tools/call",
        call("school_pull", json!({})),
    )
    .await;
    assert!(denied.get("error").is_some() || denied["result"]["isError"] == true);
    let no_token = client
        .post(&connection.url)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(no_token.status().as_u16(), 401);
    let mut wrong_connection = connection.clone();
    wrong_connection.token = "wrong".into();
    let wrong = request(&client, &wrong_connection, "ping", json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status().as_u16(), 401);
    for (header, value) in [
        ("Origin", "https://untrusted.invalid"),
        ("Host", "untrusted.invalid"),
    ] {
        let denied = request(&client, &connection, "ping", json!({}))
            .header(header, value)
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status().as_u16(), 403);
    }
    let oversized = request(
        &client,
        &connection,
        "ping",
        json!({"large":"x".repeat(MAX_REQUEST)}),
    )
    .send()
    .await
    .unwrap();
    assert_eq!(oversized.status().as_u16(), 413);
    fixture.enabled(false);
    assert_eq!(
        rpc(&client, &connection, "tools/call", call("today", json!({}))).await["result"]
            ["isError"],
        true
    );
    tokio::time::timeout(Duration::from_secs(3), server.shutdown())
        .await
        .unwrap();
    assert!(request(&client, &connection, "ping", json!({}))
        .send()
        .await
        .is_err());
    // A restart reuses the token and can bind the same port.
    let port = url::Url::parse(&connection.url).unwrap().port().unwrap();
    let restarted = AgentServer::bind(fixture.core.clone(), fixture.root.join("agent-token"), port)
        .await
        .unwrap();
    assert_eq!(restarted.connection().token, connection.token);
    assert!(
        AgentServer::bind(fixture.core.clone(), fixture.root.join("agent-token"), port)
            .await
            .is_err()
    );
    restarted.shutdown().await;
    fixture.core.shutdown().await;
}

#[tokio::test]
async fn cancelled_caller_does_not_abandon_admitted_write() {
    let fixture = Fixture::new();
    fixture.enabled(true);
    let tasks = RuntimeTasks::default();
    let permits = Arc::new(Semaphore::new(1));
    let handler = AgentHandler {
        core: fixture.core.clone(),
        calls: tasks.clone(),
        work: permits.clone(),
    };
    let mut connection = rusqlite::Connection::open(fixture.core.store.user_path()).unwrap();
    let transaction = connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let caller = tokio::spawn(async move {
        handler
            .execute(
                "weekly_event_create".into(),
                json!({
                    "term":"2026-3Q", "title":"Still committed", "days":[1],
                    "startTime":"09:00", "endTime":"09:45"
                }),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while permits.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    caller.abort();
    let _ = caller.await;
    tasks.close();
    transaction.commit().unwrap();
    tokio::time::timeout(Duration::from_secs(5), tasks.wait())
        .await
        .unwrap();
    assert_eq!(permits.available_permits(), 1);
    let snapshot = fixture.core.store.load_workspace().unwrap();
    assert_eq!(
        snapshot["settings"]["agentAccess"]["lastWrites"][0]["tool"],
        "weekly_event_create"
    );
    let schedule = fixture
        .core
        .agent
        .call("schedule", json!({"from":"2026-10-19","to":"2026-10-19"}))
        .unwrap();
    assert_eq!(schedule["events"][0]["title"], "Still committed");
    fixture.core.shutdown().await;
}

#[tokio::test]
async fn listener_and_token_follow_the_agent_access_setting() {
    let fixture = Fixture::new();
    let token_path = fixture.root.join("agent-token");
    // Hold a free port to simulate another process using it.
    let blocker = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = blocker.local_addr().unwrap().port();
    let access = AgentAccess::with(fixture.core.clone(), token_path.clone(), port);

    assert!(matches!(access.reconcile().await, AgentConnection::Off));
    assert!(!token_path.exists(), "no token while agent access is off");

    fixture.enabled(true);
    assert!(matches!(
        access.reconcile().await,
        AgentConnection::Failed { error } if error.contains("already in use")
    ));
    drop(blocker);
    let AgentConnection::Listening { url, token } = access.reconcile().await else {
        panic!("a freed port is bound on the next reconcile");
    };
    assert_eq!(url, format!("http://127.0.0.1:{port}/mcp"));
    assert_eq!(std::fs::read_to_string(&token_path).unwrap().trim(), token);
    assert!(tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .is_ok());

    fixture.enabled(false);
    assert!(matches!(access.reconcile().await, AgentConnection::Off));
    assert!(tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
        .await
        .is_err());

    fixture.enabled(true);
    access.shutdown().await;
    assert!(matches!(access.reconcile().await, AgentConnection::Off));
    fixture.core.shutdown().await;
}

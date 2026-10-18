//! The server end to end over an in-memory transport, as an MCP client sees it.

use leaf_mcp::Leaf;
use leaf_testkit::TempWiki;
use rmcp::{
    ClientHandler, ServiceExt,
    model::{CallToolRequestParams, ClientConfig},
};
use serde_json::{Value, json};

struct Client;

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

type Running = rmcp::service::RunningService<rmcp::RoleClient, Client>;

fn fixture() -> TempWiki {
    let wiki = TempWiki::new();
    wiki.write(
        "raw/notes/a.md",
        "---\nsource: direct statement\n---\n### user\n\nBeds are raised.\n\n^message-a\n",
    );
    wiki.write("index.md", "[[concepts/garden]]\n");
    wiki.write("log.md", "# Wiki Log\n");
    wiki.write(
        "_meta/vocabulary.yaml",
        "labels:\n  tags: {gardening: {}}\n",
    );
    wiki.write(
        "concepts/garden.md",
        "---\ntitle: Garden\ntype: concept\ncreated: 2026-10-01\nupdated: 2026-10-02\n\
         tags: [gardening]\nsources: [raw/notes/a.md]\nconfidence: medium\n---\n\
         # Garden\n\nAn introduction.\n\n## Beds\n\nBeds are raised. [[raw/notes/a.md#^message-a]]\n",
    );
    wiki
}

/// A connected client; the server runs until the client is dropped.
async fn client(wiki: &TempWiki) -> Running {
    let server = Leaf::open(&wiki.root(), &wiki.db()).unwrap();
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        server
            .serve(server_io)
            .await
            .unwrap()
            .waiting()
            .await
            .unwrap();
    });
    Client.serve(client_io).await.unwrap()
}

/// Call a tool: its text and whether it is an error.
async fn call(client: &Running, tool: &str, arguments: Value) -> (String, bool) {
    let arguments = arguments.as_object().cloned().unwrap_or_default();
    let result = client
        .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(arguments))
        .await
        .unwrap();
    let text = result.content[0].as_text().unwrap().text.clone();
    (text, result.is_error == Some(true))
}

#[tokio::test]
async fn the_read_tools_search_and_cite() {
    let wiki = fixture();
    let client = client(&wiki).await;
    let mut tools: Vec<String> = client
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|tool| tool.name.to_string())
        .collect();
    let mut expected = vec!["cite", "guide", "metadata", "people", "search", "stats"];
    if cfg!(feature = "editor") {
        expected.extend([
            "editor_apply",
            "editor_check",
            "editor_conformance",
            "editor_metadata",
            "editor_page",
            "editor_suggest",
        ]);
    }
    tools.sort();
    expected.sort();
    assert_eq!(tools, expected);

    let (text, error) = call(
        &client,
        "search",
        json!({"query": "beds raised", "max_passages": 2}),
    )
    .await;
    assert!(!error, "{text}");
    let response: Value = serde_json::from_str(&text).unwrap();
    let hit = &response["hits"][0];
    assert_eq!(hit["page_path"], "concepts/garden.md");
    assert_eq!(hit["citations"][0]["raw_path"], "raw/notes/a.md");

    let (text, error) = call(
        &client,
        "cite",
        json!({"reference": "raw/notes/a.md#^message-a"}),
    )
    .await;
    assert!(!error, "{text}");
    assert!(text.contains("Beds are raised."));

    let (text, error) = call(
        &client,
        "search",
        json!({"filters": [{"kind": "tag", "values": ["gardening"]}]}),
    )
    .await;
    assert!(!error, "{text}");
    assert!(text.contains("concepts/garden.md"));

    let (text, _) = call(&client, "metadata", json!({})).await;
    assert!(text.contains("tags"));
    let (text, _) = call(&client, "stats", json!({})).await;
    assert!(text.contains("\"pages\":1"));
}

#[tokio::test]
async fn the_guide_gives_the_rules_and_history_without_the_location() {
    let wiki = fixture();
    wiki.write("SCHEMA.md", "# Schema\n\nGreenhouse notes.\n");
    let mut log = String::from("# Wiki Log\n");
    for n in 1..=12 {
        log.push_str(&format!(
            "\n## [2026-10-{n:02}] update | Change {n}\n- By tester.\n"
        ));
    }
    wiki.write("log.md", &log);
    let client = client(&wiki).await;
    let (text, error) = call(&client, "guide", json!({})).await;
    assert!(!error, "{text}");
    let guide: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(guide["schema"], "# Schema\n\nGreenhouse notes.\n");
    let recent = guide["recent_log"].as_str().unwrap();
    assert!(
        recent.contains("Change 12")
            && recent.contains("Change 3")
            && !recent.contains("Change 2\n")
    );
    assert!(guide["index"].as_str().unwrap().contains("concepts/garden"));
    assert_eq!(guide["truncated"], false);
    // The agent is never told where the wiki is.
    assert!(!text.contains(&wiki.root().display().to_string()), "{text}");

    // A wiki without a schema or log still answers.
    let bare = fixture();
    std::fs::remove_file(bare.root().join("log.md")).unwrap();
    let client = self::client(&bare).await;
    let (text, _) = call(&client, "guide", json!({})).await;
    let guide: Value = serde_json::from_str(&text).unwrap();
    assert!(guide["schema"].is_null() && guide["recent_log"].is_null());
}

#[tokio::test]
async fn errors_never_name_the_wikis_folder() {
    let wiki = fixture();
    let client = client(&wiki).await;
    // The wiki breaks after the server started: the I/O error names a path.
    std::fs::remove_file(wiki.root().join("index.md")).unwrap();
    let root = wiki.root().display().to_string();
    for (tool, arguments) in [
        ("search", json!({"query": "beds"})),
        ("stats", json!({})),
        ("metadata", json!({})),
    ] {
        let (text, error) = call(&client, tool, arguments).await;
        assert!(error, "{tool}: {text}");
        assert!(text.contains("index.md"), "{tool}: {text}");
        assert!(!text.contains(&root), "{tool} leaked the folder: {text}");
        assert!(!text.contains("/tmp/"), "{tool} leaked a path: {text}");
    }
}

#[tokio::test]
async fn bad_requests_are_tool_errors_the_model_can_read() {
    let wiki = fixture();
    let client = client(&wiki).await;
    let (text, error) = call(&client, "search", json!({})).await;
    assert!(error && text.contains("at least one filter"), "{text}");
    let (text, error) = call(&client, "cite", json!({"reference": "not-a-reference"})).await;
    assert!(error && text.contains("raw/<path>"), "{text}");
    let (text, error) = call(
        &client,
        "cite",
        json!({"reference": "raw/notes/a.md#^message-zzz"}),
    )
    .await;
    assert!(error, "{text}");
}

#[cfg(feature = "editor")]
#[tokio::test]
async fn the_editor_tools_check_and_apply_batches() {
    let wiki = fixture();
    let client = client(&wiki).await;
    let batch = |tag: &str| {
        format!(
            "agent: curator\nmodel: small-8b\nkind: update\nsummary: MCP\nops:\n\
             - {{op: add_passage, page: concepts/garden, under: Beds, text: Beds face south., \
             citations: ['raw/notes/a.md#^message-a']}}\n\
             - {{op: add_metadata, page: concepts/garden, key: tags, values: [{tag}]}}\n"
        )
    };
    let page = || wiki.read("concepts/garden.md");
    let before = page();

    // A rejected batch is a tool error naming the problem, and writes nothing.
    let (text, error) = call(
        &client,
        "editor_apply",
        json!({"batch": batch("gardnening")}),
    )
    .await;
    assert!(error, "{text}");
    assert!(text.contains("gardening"), "{text}");
    assert_eq!(page(), before);

    let (text, error) = call(
        &client,
        "editor_check",
        json!({"batch": batch("gardening")}),
    )
    .await;
    assert!(!error, "{text}");
    assert_eq!(page(), before);

    let (text, error) = call(
        &client,
        "editor_apply",
        json!({"batch": batch("gardening")}),
    )
    .await;
    assert!(!error, "{text}");
    assert!(text.contains("\"applied\":true"));
    assert!(page().contains("Beds face south. [[raw/notes/a.md#^message-a]]"));
    // The read tools see the change at once.
    let (text, _) = call(&client, "search", json!({"query": "beds face south"})).await;
    assert!(text.contains("Beds face south."), "{text}");

    let (text, error) = call(&client, "editor_page", json!({"page": "concepts/garden"})).await;
    assert!(!error && text.contains("concepts/garden.md#p-"), "{text}");
    let (text, _) = call(&client, "editor_suggest", json!({"key": "persons"})).await;
    assert!(text.contains("related-people"), "{text}");
    let (text, error) = call(&client, "editor_check", json!({"batch": "not: a batch"})).await;
    assert!(error && text.contains("invalid batch"), "{text}");
    let (text, _) = call(&client, "editor_conformance", json!({})).await;
    assert!(text.contains("pages"), "{text}");
    let (text, _) = call(&client, "editor_metadata", json!({})).await;
    assert!(text.contains("gardening"), "{text}");
}

/// The batch format the editor tools document, with the example it ends on.
#[cfg(feature = "editor")]
const BATCH_FORMAT: &str = include_str!("../src/batch-format.md");

#[cfg(feature = "editor")]
#[tokio::test]
async fn the_documented_batch_format_is_in_the_tools_and_its_example_is_accepted() {
    let wiki = fixture();
    let client = client(&wiki).await;

    // The description of editor_apply carries the format (and the example).
    let tools = client.list_all_tools().await.unwrap();
    let apply = tools.iter().find(|t| t.name == "editor_apply").unwrap();
    let schema = serde_json::to_string(&apply.input_schema).unwrap();
    assert!(schema.contains("Operations (`?` is optional)"), "{schema}");
    assert!(schema.contains("create_page"), "{schema}");

    // A wiki the example can apply to: three linked pages (the tag is declared).
    let page = |slug: &str, a: &str, b: &str| {
        format!(
            "  - op: create_page\n    type: concept\n    slug: {slug}\n    title: {slug}\n    summary: About {slug}\n    \
             metadata: {{tags: [gardening], confidence: medium}}\n    intro:\n      - text: See [[concepts/{a}]] and [[concepts/{b}]].\n    \
             sections:\n      - heading: Practice\n        passages:\n          - text: A claim.\n            citations: [\"raw/notes/a.md#^message-a\"]\n"
        )
    };
    let seed = format!(
        "agent: t\nmodel: t\nkind: update\nsummary: Seed\nops:\n{}{}{}",
        page("light", "soil", "water"),
        page("soil", "light", "water"),
        page("water", "light", "soil"),
    );
    let (text, error) = call(&client, "editor_apply", json!({"batch": seed})).await;
    assert!(!error, "{text}");

    let start = BATCH_FORMAT.find("```yaml\n").unwrap() + "```yaml\n".len();
    let example = &BATCH_FORMAT[start..start + BATCH_FORMAT[start..].find("```").unwrap()];
    let (text, error) = call(&client, "editor_check", json!({"batch": example})).await;
    assert!(!error, "the documented example is rejected: {text}");
}

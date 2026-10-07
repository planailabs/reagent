//! A running reagent against a scripted model: each task (by its title)
//! has a queue of replies; a reply sees the request.
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use axum::{Json, Router, body::Body, extract::State, response::Response, routing::post};
use serde_json::{Value, json};

pub type Reply = Box<dyn Fn(&Value) -> Vec<String> + Send + Sync>;

#[derive(Default)]
pub struct Brain {
    queues: HashMap<String, VecDeque<Reply>>,
    pub requests: Vec<Value>,
}

/// The task a request is from: its first message says `Task: <title>`.
pub fn title_of(body: &Value) -> String {
    body["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "user").find_map(|m| m["content"].as_str().and_then(|c| c.strip_prefix("Task: ")).map(|c| c.lines().next().unwrap_or("").to_string())).unwrap_or_default()
}

async fn handle(State(brain): State<Arc<Mutex<Brain>>>, Json(body): Json<Value>) -> Response {
    let title = title_of(&body);
    let reply = {
        let mut b = brain.lock().unwrap();
        b.requests.push(body.clone());
        b.queues.get_mut(&title).and_then(VecDeque::pop_front)
    };
    let Some(reply) = reply else {
        return Response::builder().status(500).body(Body::from(format!("no scripted reply for task {title:?}"))).unwrap();
    };
    let chunks = reply(&body);
    let s: String = chunks.into_iter().map(|c| format!("data: {c}\n\n")).collect();
    Response::builder().header("content-type", "text/event-stream").body(Body::from(s)).unwrap()
}

pub fn text(s: &str) -> Vec<String> {
    vec![
        json!({"choices":[{"delta":{"content":s}}]}).to_string(),
        json!({"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":20}}).to_string(),
        "[DONE]".into(),
    ]
}

/// A tool call (`fs.read` is sent as `fs__read`, as models see it).
pub fn call(id: &str, tool: &str, args: Value) -> Vec<String> {
    vec![
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":tool.replace('.', "__"),"arguments":args.to_string()}}]}}]}).to_string(),
        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":100,"completion_tokens":20}}).to_string(),
        "[DONE]".into(),
    ]
}

/// The last tool result in a request.
pub fn last_result(body: &Value) -> String {
    body["messages"].as_array().unwrap().iter().rev().find(|m| m["role"] == "tool").map(|m| m["content"].as_str().unwrap_or("").to_string()).unwrap_or_default()
}

/// Every message's text in a request, joined.
pub fn all_text(body: &Value) -> String {
    body["messages"].as_array().unwrap().iter().filter_map(|m| m["content"].as_str()).collect::<Vec<_>>().join("\n")
}

pub struct R {
    pub run: reagent::Running,
    pub brain: Arc<Mutex<Brain>>,
    pub data: tempfile::TempDir,
    pub project: tempfile::TempDir,
    pub http: reqwest::Client,
}

static ENV: Once = Once::new();

pub fn git(dir: &Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

pub async fn start() -> R {
    start_in(tempfile::tempdir().unwrap(), "").await
}

/// reagent with `extra` appended to reagent.hcl, a project "site" (a git repo).
pub async fn start_in(data: tempfile::TempDir, extra: &str) -> R {
    ENV.call_once(|| unsafe {
        std::env::set_var(reagent_tools::cluster::TOKEN_ENV, "test-token");
        std::env::set_var("HOME", std::env::temp_dir().join("reagent-test-home"));
    });
    let brain = Arc::new(Mutex::new(Brain::default()));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", l.local_addr().unwrap());
    let r = Router::new().route("/v1/chat/completions", post(handle)).with_state(brain.clone());
    tokio::spawn(async move { axum::serve(l, r).await.unwrap() });
    std::fs::write(
        data.path().join("reagent.hcl"),
        format!("listen = \"127.0.0.1:0\"\nprovider \"mock\" {{\n  base_url = \"{url}\"\n}}\nprofile \"default\" {{\n  provider = \"mock\"\n  model = \"m\"\n  price = {{ input = 1.0, output = 2.0 }}\n}}\n{extra}"),
    )
    .unwrap();
    let project = tempfile::tempdir().unwrap();
    let p = project.path();
    git(p, &["init", "-q", "-b", "main"]);
    git(p, &["config", "user.email", "t@t"]);
    git(p, &["config", "user.name", "t"]);
    std::fs::write(p.join("a.txt"), "hello world\n").unwrap();
    git(p, &["add", "."]);
    git(p, &["commit", "-q", "-m", "init"]);
    let run = reagent::up(reagent::Opts { data: data.path().into(), listen: None, in_process_supervisor: true, exe: PathBuf::new(), dist: PathBuf::from("/nonexistent") }).await.unwrap();
    let mut proj = reagent_store::Project::new("site", "Site", &p.display().to_string());
    proj.merge = "approve".into();
    run.app.put_project(proj).await.unwrap();
    R { run, brain, data, project, http: reqwest::Client::new() }
}

impl R {
    pub fn push(&self, title: &str, r: impl Fn(&Value) -> Vec<String> + Send + Sync + 'static) {
        self.brain.lock().unwrap().queues.entry(title.into()).or_default().push_back(Box::new(r));
    }

    pub fn requests(&self, title: &str) -> Vec<Value> {
        self.brain.lock().unwrap().requests.iter().filter(|b| title_of(b) == title).cloned().collect()
    }

    pub async fn start_task(&self, title: &str, prompt: &str) -> reagent_store::Task {
        self.run.app.start_task(reagent_tools::app::StartTask { project: "site".into(), title: title.into(), prompt: prompt.into(), ..Default::default() }).await.unwrap()
    }

    pub async fn task(&self, id: &str) -> reagent_store::Task {
        self.run.app.task(id).await.unwrap()
    }

    /// Waits until `f` holds for the task.
    pub async fn until(&self, id: &str, what: &str, f: impl Fn(&reagent_store::Task) -> bool) -> reagent_store::Task {
        for _ in 0..600 {
            let t = self.task(id).await;
            if f(&t) {
                return t;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let t = self.task(id).await;
        panic!("timed out waiting for {what}: {t:?}\nrequests: {:#?}", self.requests(&t.title).iter().map(|r| r["messages"].as_array().unwrap().last().cloned()).collect::<Vec<_>>());
    }

    pub async fn done(&self, id: &str) -> reagent_store::Task {
        self.until(id, "done", |t| t.state == "done").await
    }
}

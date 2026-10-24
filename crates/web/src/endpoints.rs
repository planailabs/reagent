//! Endpoints declared once: a method, a path, a tool name, a description and a
//! handler over one typed input. Each is served as a route of the web API and
//! as a tool of the MCP API, with the same handler and the same input (its
//! JSON schema is the tool's). The input gathers the path's parameters, the
//! query and the JSON body (a body that isn't an object comes as `body`).

use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{Method, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{MethodFilter, MethodRouter, on};
use futures::future::BoxFuture;
use rmcp::model::JsonObject;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::S;
use crate::api::{E, R};

/// Who calls: the person (the web UI) or an agent outside (the MCP API, with a token).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Who {
    Person,
    Agent,
}

type Call = Arc<dyn Fn(S, Who, Value) -> BoxFuture<'static, Result<Value, E>> + Send + Sync>;

pub struct Endpoint {
    pub method: Method,
    pub path: &'static str,
    pub tool: &'static str,
    pub description: &'static str,
    pub schema: Arc<JsonObject>,
    call: Call,
}

impl Endpoint {
    pub async fn call(&self, s: S, who: Who, input: Value) -> Result<Value, E> {
        (self.call)(s, who, input).await
    }
}

#[derive(Default, Clone)]
pub struct Endpoints(pub Vec<Arc<Endpoint>>);

impl Endpoints {
    fn add<I, F, Fut>(&mut self, method: Method, path: &'static str, tool: &'static str, description: &'static str, f: F) -> &mut Self
    where
        I: DeserializeOwned + JsonSchema + Send + 'static,
        F: Fn(S, Who, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
    {
        assert!(!self.0.iter().any(|e| e.tool == tool), "two endpoints called {tool}");
        let f = Arc::new(f);
        let call: Call = Arc::new(move |s, who, v| {
            let f = f.clone();
            Box::pin(async move {
                let input: I = serde_json::from_value(v).map_err(|e| E(StatusCode::BAD_REQUEST, format!("bad input: {e}")))?;
                f(s, who, input).await.map(|j| j.0)
            })
        });
        self.0.push(Arc::new(Endpoint { method, path, tool, description, schema: rmcp::handler::server::common::schema_for_type::<I>(), call }));
        self
    }

    pub fn get<I, F, Fut>(&mut self, path: &'static str, tool: &'static str, description: &'static str, f: F) -> &mut Self
    where
        I: DeserializeOwned + JsonSchema + Send + 'static,
        F: Fn(S, Who, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
    {
        self.add(Method::GET, path, tool, description, f)
    }

    pub fn post<I, F, Fut>(&mut self, path: &'static str, tool: &'static str, description: &'static str, f: F) -> &mut Self
    where
        I: DeserializeOwned + JsonSchema + Send + 'static,
        F: Fn(S, Who, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
    {
        self.add(Method::POST, path, tool, description, f)
    }

    pub fn put<I, F, Fut>(&mut self, path: &'static str, tool: &'static str, description: &'static str, f: F) -> &mut Self
    where
        I: DeserializeOwned + JsonSchema + Send + 'static,
        F: Fn(S, Who, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
    {
        self.add(Method::PUT, path, tool, description, f)
    }

    pub fn delete<I, F, Fut>(&mut self, path: &'static str, tool: &'static str, description: &'static str, f: F) -> &mut Self
    where
        I: DeserializeOwned + JsonSchema + Send + 'static,
        F: Fn(S, Who, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = R> + Send + 'static,
    {
        self.add(Method::DELETE, path, tool, description, f)
    }

    /// The web API's routes for them (the person calls).
    pub fn routes(&self) -> Router<S> {
        let mut by_path: BTreeMap<&str, MethodRouter<S>> = BTreeMap::new();
        for ep in &self.0 {
            let filter = MethodFilter::try_from(ep.method.clone()).expect("a method axum routes");
            let e = ep.clone();
            let handler = move |State(s): State<S>, path: Option<Path<HashMap<String, String>>>, Query(query): Query<Vec<(String, String)>>, body: Bytes| async move {
                match input(&e.schema, path.map(|p| p.0).unwrap_or_default(), query, &body) {
                    Ok(v) => match e.call(s, Who::Person, v).await {
                        Ok(v) => axum::Json(v).into_response(),
                        Err(err) => err.into_response(),
                    },
                    Err(err) => err.into_response(),
                }
            };
            let mr = by_path.remove(ep.path).unwrap_or_default();
            by_path.insert(ep.path, mr.merge(on(filter, handler)));
        }
        by_path.into_iter().fold(Router::new(), |r, (p, mr)| r.route(p, mr))
    }
}

/// A request's input: the body's fields, the query's, then the path's (they win).
/// Query and path values are text: a field the schema types as a number or a
/// boolean is parsed.
fn input(schema: &JsonObject, path: HashMap<String, String>, query: Vec<(String, String)>, body: &[u8]) -> Result<Value, E> {
    let mut m = match body.iter().all(u8::is_ascii_whitespace) {
        true => Map::new(),
        false => match serde_json::from_slice(body).map_err(|e| E(StatusCode::BAD_REQUEST, format!("the body isn't JSON: {e}")))? {
            Value::Object(m) => m,
            v => Map::from_iter([("body".to_string(), v)]),
        },
    };
    for (k, v) in query.into_iter().chain(path) {
        let v = coerce(schema, &k, v);
        m.insert(k, v);
    }
    Ok(Value::Object(m))
}

fn coerce(schema: &JsonObject, key: &str, v: String) -> Value {
    let typed = |t: &Value| matches!(t.as_str(), Some("integer" | "number" | "boolean"));
    let is_typed = match schema.get("properties").and_then(|p| p.get(key)).and_then(|p| p.get("type")) {
        Some(Value::Array(ts)) => ts.iter().any(typed),
        Some(t) => typed(t),
        None => false,
    };
    if is_typed { serde_json::from_str(&v).unwrap_or(Value::String(v)) } else { Value::String(v) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(serde::Deserialize, JsonSchema)]
    #[allow(dead_code)]
    struct In {
        id: i64,
        active: Option<bool>,
        name: String,
    }

    #[test]
    fn path_query_and_body_make_one_typed_input() {
        let schema = rmcp::handler::server::common::schema_for_type::<In>();
        let path = HashMap::from([("id".to_string(), "7".to_string())]);
        let v = input(&schema, path, vec![("active".into(), "true".into()), ("name".into(), "12".into())], br#"{"id": 1, "x": [1]}"#).unwrap();
        // The path wins; text stays text where the schema says string.
        assert_eq!(v, json!({"id": 7, "active": true, "name": "12", "x": [1]}));
        assert_eq!(input(&schema, HashMap::new(), vec![], b"[1, 2]").unwrap(), json!({"body": [1, 2]}));
        assert_eq!(input(&schema, HashMap::new(), vec![], b"").unwrap(), json!({}));
        assert!(input(&schema, HashMap::new(), vec![], b"{nope").is_err());
    }
}

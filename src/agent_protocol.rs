use std::io::{self, BufRead, BufReader, Write};

use serde_json::{Map, Value, json};

use crate::launch_url::literal_http_url;

const MAX_LINE_BYTES: usize = 256 * 1024;
const MAX_URL_BYTES: usize = 4096;
// These initialize revisions both use single-message frames and structured tool results.
const PROTOCOL_VERSION: &str = "2025-11-25";
const SUPPORTED_VERSIONS: &[&str] = &[PROTOCOL_VERSION, "2025-06-18"];

const OPEN: &str = "flit_open";
const LIST: &str = "flit_list_tabs";
const SNAPSHOT: &str = "flit_snapshot";
const FOLLOW: &str = "flit_follow_link";
const CLOSE: &str = "flit_close_tab";
const SIGN_IN: &str = "flit_sign_in";

#[derive(Clone, Debug)]
pub(crate) enum Command {
    Open { url: String },
    List,
    Snapshot { tab: u32 },
    Follow { tab: u32, snapshot: u32, link: u32 },
    Close { tab: u32 },
    SignIn { tab: u32 },
}

pub(crate) fn serve(
    mut execute: impl FnMut(Command) -> Result<serde_json::Value, String>,
) -> std::io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    serve_io(BufReader::new(stdin.lock()), stdout.lock(), &mut execute)
}

fn serve_io(
    mut input: impl BufRead,
    mut output: impl Write,
    execute: &mut impl FnMut(Command) -> Result<Value, String>,
) -> io::Result<()> {
    let mut line = Vec::with_capacity(MAX_LINE_BYTES + 1);
    while let Some(frame) = read_frame(&mut input, &mut line)? {
        let reply = match frame {
            Frame::Line => match serde_json::from_slice(&line) {
                Ok(message) => dispatch(&message, execute),
                Err(_) => Some(rpc_error(&Value::Null, -32700, "Invalid UTF-8 or JSON")),
            },
            Frame::Oversized => Some(rpc_error(
                &Value::Null,
                -32700,
                "Input line exceeds 256 KiB",
            )),
            Frame::Unterminated => Some(rpc_error(
                &Value::Null,
                -32700,
                "Input line requires a newline",
            )),
        };
        if let Some(reply) = reply {
            serde_json::to_writer(&mut output, &reply).map_err(io::Error::from)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}

enum Frame {
    Line,
    Oversized,
    Unterminated,
}

fn read_frame(input: &mut impl BufRead, line: &mut Vec<u8>) -> io::Result<Option<Frame>> {
    line.clear();
    let mut oversized = false;
    loop {
        let chunk = match input.fill_buf() {
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if chunk.is_empty() {
            return Ok(if oversized || line.len() > MAX_LINE_BYTES {
                Some(Frame::Oversized)
            } else if line.is_empty() {
                None
            } else {
                Some(Frame::Unterminated)
            });
        }

        let newline = chunk.iter().position(|&byte| byte == b'\n');
        let count = newline.unwrap_or(chunk.len());
        // Reserve one byte for CRLF. After overflow, consume through LF without copying.
        if !oversized {
            if count <= MAX_LINE_BYTES + 1 - line.len() {
                line.extend_from_slice(&chunk[..count]);
            } else {
                oversized = true;
            }
        }
        input.consume(count + usize::from(newline.is_some()));
        if newline.is_some() {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(Some(if oversized || line.len() > MAX_LINE_BYTES {
                Frame::Oversized
            } else {
                Frame::Line
            }));
        }
    }
}

type RpcResult = Result<Value, (i32, &'static str)>;

fn dispatch(
    message: &Value,
    execute: &mut impl FnMut(Command) -> Result<Value, String>,
) -> Option<Value> {
    let raw_id = message.get("id");
    let id = raw_id.filter(|id| id.is_string() || id.is_i64() || id.is_u64());
    let method = message.get("method").and_then(Value::as_str);
    let is_result = message.get("result").is_some();
    let is_error = message.get("error").is_some();

    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(rpc_error(
            id.unwrap_or(&Value::Null),
            -32600,
            "Invalid Request",
        ));
    }
    if message.get("method").is_none() && raw_id.is_some() && (is_result ^ is_error) {
        // We never issue requests; ignore unsolicited responses to avoid error loops.
        return None;
    }
    let Some(method) = method.filter(|_| !is_result && !is_error) else {
        return Some(rpc_error(
            id.unwrap_or(&Value::Null),
            -32600,
            "Invalid Request",
        ));
    };
    raw_id?; // No notification may dispatch a command or receive a reply.
    let Some(id) = id else {
        return Some(rpc_error(&Value::Null, -32600, "Invalid request ID"));
    };

    let params = match message.get("params") {
        None => None,
        Some(Value::Object(params)) => Some(params),
        Some(_) => return Some(rpc_error(id, -32602, "Params must be an object")),
    };
    let result = match method {
        "initialize" => initialize(params),
        "ping" => Ok(json!({})),
        "tools/list" if params.is_some_and(|params| params.contains_key("cursor")) => {
            Err((-32602, "Invalid tools/list cursor"))
        }
        "tools/list" => Ok(tool_catalog()),
        "tools/call" => call_tool(params, execute),
        _ => Err((-32601, "Method not found")),
    };
    Some(match result {
        Ok(result) => {
            let mut reply = json!({"jsonrpc":"2.0", "id":id});
            reply["result"] = result;
            reply
        }
        Err((code, message)) => rpc_error(id, code, message),
    })
}

fn rpc_error(id: &Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id":id, "error":{"code":code, "message":message}})
}

fn initialize(params: Option<&Map<String, Value>>) -> RpcResult {
    let requested = params
        .and_then(|params| params.get("protocolVersion"))
        .and_then(Value::as_str)
        .filter(|version| !version.is_empty())
        .ok_or((-32602, "Expected a protocolVersion string"))?;
    let version = if SUPPORTED_VERSIONS.contains(&requested) {
        requested
    } else {
        PROTOCOL_VERSION
    };
    Ok(json!({
        "protocolVersion":version,
        "capabilities":{"tools":{"listChanged":false}},
        "serverInfo":{"name":"flit-browser","version":env!("CARGO_PKG_VERSION")},
        "instructions":"Use isolated agent tabs for public browsing. Use flit_sign_in only with explicit Flit Vault enrollment for the tab's current approved HTTPS origin and a supported login form. Never request or pass credentials. Ordinary form submission, chat interaction, and arbitrary scripts are unavailable. A submitted status with reason destination_verified means the same-origin destination returned HTTP 2xx with no visible login or OTP prompt. Use a fresh flit_snapshot to look for site-specific confirmation before reporting that the user is signed in. If the tool returns action_required, report the reason."
    }))
}

fn tool_catalog() -> Value {
    fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
        json!({
            "name":name,
            "description":description,
            "inputSchema":{
                "type":"object",
                "properties":properties,
                "required":required,
                "additionalProperties":false
            }
        })
    }
    let id = json!({"type":"integer","minimum":0,"maximum":u32::MAX});
    json!({"tools":[
        tool(OPEN, "Open a public HTTP(S) URL in a new isolated agent tab. The URL must be at most 4096 bytes and contain no credentials.",
            json!({"url":{"type":"string","minLength":1,"maxLength":MAX_URL_BYTES}}), &["url"]),
        tool(LIST, "List isolated agent tabs for public pages, including each tab ID, label, and status.",
            json!({}), &[]),
        tool(SNAPSHOT, "Read a public page in an isolated agent tab. Returns page text, a snapshot version (snapshot), and links with id, text, and url.",
            json!({"tab":id}), &["tab"]),
        tool(FOLLOW, "Navigate an isolated agent tab using a public link ID from the specified snapshot. No arbitrary clicks, form submission, or chat interaction.",
            json!({"tab":id,"snapshot":id,"link":id}), &["tab", "snapshot", "link"]),
        tool(CLOSE, "Close an isolated agent tab used for a public page.",
            json!({"tab":id}), &["tab"]),
        tool(SIGN_IN, "Attempt sign-in on the current page of an isolated agent tab. Requires a supported login form and a saved Flit Vault credential explicitly enrolled for the tab's current approved HTTPS origin. Supply only the tab ID. Returns status submitted with reason destination_verified after a same-origin destination returns HTTP 2xx with no visible login or OTP prompt, or action_required with a reason. Use a fresh flit_snapshot to confirm sign-in on the site.",
            json!({"tab":id}), &["tab"])
    ]})
}

fn call_tool(
    params: Option<&Map<String, Value>>,
    execute: &mut impl FnMut(Command) -> Result<Value, String>,
) -> RpcResult {
    let params = params.ok_or((-32602, "Missing tool call params"))?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((-32602, "Expected a tool name string"))?;
    if params
        .keys()
        .any(|key| !matches!(key.as_str(), "name" | "arguments" | "_meta"))
        || params.get("_meta").is_some_and(|meta| !meta.is_object())
    {
        return Err((-32602, "Unsupported tool call params"));
    }
    let empty = Map::new();
    let args = match params.get("arguments") {
        None => &empty,
        Some(Value::Object(args)) => args,
        Some(_) => return Err((-32602, "Tool arguments must be an object")),
    };
    let number = |key| args.get(key)?.as_u64()?.try_into().ok();
    let command = match (name, args.len()) {
        (OPEN, 1) => args
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| url.len() <= MAX_URL_BYTES)
            .and_then(literal_http_url)
            .map(|url| Command::Open {
                url: url.to_owned(),
            }),
        (LIST, 0) => Some(Command::List),
        (SNAPSHOT, 1) => number("tab").map(|tab| Command::Snapshot { tab }),
        (FOLLOW, 3) => match (number("tab"), number("snapshot"), number("link")) {
            (Some(tab), Some(snapshot), Some(link)) => Some(Command::Follow {
                tab,
                snapshot,
                link,
            }),
            _ => None,
        },
        (CLOSE, 1) => number("tab").map(|tab| Command::Close { tab }),
        (SIGN_IN, 1) => number("tab").map(|tab| Command::SignIn { tab }),
        (OPEN | LIST | SNAPSHOT | FOLLOW | CLOSE | SIGN_IN, _) => None,
        _ => return Err((-32602, "Unknown tool")),
    };

    Ok(match command {
        Some(command) => tool_result(execute(command)),
        None => tool_result(Err(
            "Invalid tool arguments; see the tool description and input schema.".into(),
        )),
    })
}

fn tool_result(result: Result<Value, String>) -> Value {
    match result {
        Ok(value) => {
            let text = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned);
            let mut result = json!({"content":[{"type":"text","text":text}],"isError":false});
            if value.is_object() {
                result["structuredContent"] = value;
            }
            result
        }
        Err(message) => json!({"content":[{"type":"text","text":message}],"isError":true}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn call(id: u32, name: &str, arguments: Value) -> Value {
        json!({"jsonrpc":"2.0", "id":id, "method":"tools/call",
               "params":{"name":name, "arguments":arguments}})
    }

    fn transcript(
        input: &[u8],
        mut execute: impl FnMut(Command) -> Result<Value, String>,
    ) -> Vec<Value> {
        let mut output = Vec::new();
        serve_io(
            BufReader::with_capacity(127, Cursor::new(input)),
            &mut output,
            &mut execute,
        )
        .unwrap();
        output
            .split_inclusive(|&byte| byte == b'\n')
            .map(|line| {
                assert_eq!(line.last(), Some(&b'\n'));
                serde_json::from_slice(line).unwrap()
            })
            .collect()
    }

    fn requests(
        input: &[Value],
        execute: impl FnMut(Command) -> Result<Value, String>,
    ) -> Vec<Value> {
        let mut lines = Vec::new();
        for request in input {
            serde_json::to_writer(&mut lines, request).unwrap();
            lines.push(b'\n');
        }
        transcript(&lines, execute)
    }

    #[test]
    fn negotiates_and_lists_without_starting_the_engine() {
        for (requested, negotiated) in [
            ("2025-11-25", "2025-11-25"),
            ("2025-06-18", "2025-06-18"),
            ("2025-03-26", "2025-11-25"),
        ] {
            let replies = requests(
                &[
                    json!({"jsonrpc":"2.0","id":"init","method":"initialize","params":{
                        "protocolVersion":requested,"capabilities":{},
                        "clientInfo":{"name":"test","version":"1"}
                    }}),
                    json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                    json!({"jsonrpc":"2.0","id":2,"method":"ping"}),
                    json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
                ],
                |_| panic!("control messages must not invoke the engine"),
            );
            assert_eq!(replies.len(), 3);
            assert_eq!(replies[0]["id"], "init");
            assert_eq!(replies[0]["result"]["protocolVersion"], negotiated);
            assert_eq!(
                replies[0]["result"]["capabilities"]["tools"]["listChanged"],
                false
            );
            assert_eq!(replies[1], json!({"jsonrpc":"2.0","id":2,"result":{}}));
            assert_eq!(replies[2]["id"], 3);
            let tools = replies[2]["result"]["tools"].as_array().unwrap();
            let names: Vec<_> = tools
                .iter()
                .map(|tool| tool["name"].as_str().unwrap())
                .collect();
            assert_eq!(
                names,
                [
                    "flit_open",
                    "flit_list_tabs",
                    "flit_snapshot",
                    "flit_follow_link",
                    "flit_close_tab",
                    "flit_sign_in"
                ]
            );
            assert!(
                tools
                    .iter()
                    .all(|tool| tool["inputSchema"]["additionalProperties"] == false)
            );
            assert_eq!(
                tools[5]["inputSchema"],
                json!({
                    "type":"object",
                    "properties":{"tab":{"type":"integer","minimum":0,"maximum":u32::MAX}},
                    "required":["tab"],
                    "additionalProperties":false
                })
            );
        }
    }

    #[test]
    fn rejects_invalid_sign_in_arguments_without_dispatch() {
        for arguments in [
            json!({}),
            json!({"tab":-1}),
            json!({"tab":1.5}),
            json!({"tab":"1"}),
            json!({"tab":u64::from(u32::MAX) + 1}),
            json!({"tab":1,"credentials":{}}),
            json!({"tab":1,"selector":"form"}),
            json!({"tab":1,"url":"https://example.invalid/"}),
        ] {
            let request = call(1, "flit_sign_in", arguments);
            let reply =
                dispatch(&request, &mut |_| panic!("invalid sign-in call dispatched")).unwrap();
            assert_eq!(reply["result"]["isError"], true, "{request}");
        }
    }

    #[test]
    fn rejects_invalid_calls_and_ignores_notifications_without_dispatch() {
        let mut notification = call(1, "flit_open", json!({"url":"https://example.com/"}));
        notification.as_object_mut().unwrap().remove("id");
        let replies = requests(
            &[
                notification,
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
                json!({"jsonrpc":"2.0","id":null,"method":"tools/call","params":{"name":"flit_list_tabs"}}),
                json!({"jsonrpc":"2.0","id":2,"method":"unknown"}),
                call(3, "flit_eval", json!({})),
                call(4, "flit_list_tabs", Value::Null),
                call(5, "flit_open", json!({"url":"javascript:void(0)"})),
                call(6, "flit_open", json!({"url":"https://@example.com/"})),
                call(
                    7,
                    "flit_open",
                    json!({"url":"https://example.com/","headers":{}}),
                ),
                call(8, "flit_snapshot", json!({"tab":4294967296_u64})),
                call(
                    9,
                    "flit_follow_link",
                    json!({"tab":1,"snapshot":2,"link":-1}),
                ),
                call(10, "flit_close_tab", json!({"tab":"1"})),
                call(
                    11,
                    "flit_open",
                    json!({"url":format!("https://example.com/{}", "a".repeat(4096))}),
                ),
            ],
            |_| panic!("invalid calls and notifications must not invoke the engine"),
        );
        assert_eq!(replies.len(), 11);
        assert_eq!(replies[0]["id"], Value::Null);
        assert_eq!(replies[0]["error"]["code"], -32600);
        for (reply, code) in replies[1..4].iter().zip([-32601, -32602, -32602]) {
            assert_eq!(reply["error"]["code"], code);
        }
        for reply in &replies[4..] {
            assert_eq!(reply["result"]["isError"], true);
            assert_eq!(reply["result"]["content"][0]["type"], "text");
        }
    }

    #[test]
    fn accepts_the_line_limit_and_drains_oversized_frames() {
        const LIMIT: usize = 256 * 1024;
        let ping = json!({"jsonrpc":"2.0","id":1,"method":"ping"});
        let command = call(2, "flit_list_tabs", json!({}));
        let mut input = serde_json::to_vec(&ping).unwrap();
        input.resize(LIMIT, b' ');
        input.extend_from_slice(b"\r\n");

        // A complete command prefix on an oversized line must never dispatch.
        let start = input.len();
        serde_json::to_writer(&mut input, &command).unwrap();
        input.resize(start + LIMIT + 1, b' ');
        input.push(b'\n');
        let start = input.len();
        serde_json::to_writer(&mut input, &command).unwrap();
        input.resize(start + 3 * LIMIT, b' ');
        serde_json::to_writer(&mut input, &command).unwrap();
        input.push(b'\n');

        input.extend_from_slice(b"{\"jsonrpc\":\"2.0\",\"id\":\"\xff\",\"method\":\"ping\"}\n");
        serde_json::to_writer(&mut input, &ping).unwrap();
        input.push(b'\n');
        serde_json::to_writer(&mut input, &command).unwrap();

        let replies = transcript(&input, |_| panic!("invalid frame dispatched a command"));
        assert_eq!(replies.len(), 6);
        assert_eq!(replies[0]["id"], 1);
        assert_eq!(replies[4], replies[0]);
        for index in [1, 2, 3, 5] {
            assert_eq!(replies[index]["id"], Value::Null);
            assert_eq!(replies[index]["error"]["code"], -32700);
        }
    }

    #[test]
    fn dispatches_typed_commands_and_wraps_results() {
        let url = "HTTPS://example.com/%2F?q=a@b";
        let snapshot = json!({"snapshot":2,"text":"첫 줄\n다음 줄",
                             "links":[{"id":3,"text":"example","url":url}]});
        let sign_in = json!({"tab":u32::MAX,"status":"submitted","reason":"destination_verified"});
        let mut executed = 0;
        let replies = requests(
            &[
                call(1, "flit_open", json!({"url":url})),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"flit_list_tabs"}}),
                call(3, "flit_snapshot", json!({"tab":u32::MAX})),
                call(
                    4,
                    "flit_follow_link",
                    json!({"tab":1,"snapshot":2,"link":3}),
                ),
                call(5, "flit_close_tab", json!({"tab":1})),
                call(6, "flit_sign_in", json!({"tab":u32::MAX})),
            ],
            |command| {
                executed += 1;
                match (executed, command) {
                    (1, Command::Open { url: actual }) if actual == url => Ok(json!({"tab":1})),
                    (2, Command::List) => Ok(json!({"tabs":[]})),
                    (3, Command::Snapshot { tab: u32::MAX }) => Ok(snapshot.clone()),
                    (
                        4,
                        Command::Follow {
                            tab: 1,
                            snapshot: 2,
                            link: 3,
                        },
                    ) => Err("stale snapshot".into()),
                    (5, Command::Close { tab: 1 }) => Ok(json!("closed")),
                    (6, Command::SignIn { tab: u32::MAX }) => Ok(sign_in.clone()),
                    other => panic!("unexpected dispatch: {other:?}"),
                }
            },
        );
        assert_eq!(executed, 6);
        assert_eq!(replies[2]["result"]["isError"], false);
        assert_eq!(replies[2]["result"]["structuredContent"], snapshot);
        assert_eq!(
            serde_json::from_str::<Value>(
                replies[2]["result"]["content"][0]["text"].as_str().unwrap()
            )
            .unwrap(),
            snapshot,
        );
        assert_eq!(replies[3]["result"]["isError"], true);
        assert_eq!(replies[3]["result"]["content"][0]["text"], "stale snapshot");
        assert!(replies[3]["result"].get("structuredContent").is_none());
        assert_eq!(replies[4]["result"]["content"][0]["text"], "closed");
        assert!(replies[4]["result"].get("structuredContent").is_none());
        assert_eq!(replies[5]["result"]["isError"], false);
        assert_eq!(replies[5]["result"]["structuredContent"], sign_in);
    }
}

//! Dev tool: run one Pathfinder query with the saved Catalogue login and print the answer's *shape*.
//!
//!     cargo run -p singularityd --example probe -- <operation> '<variables json>' [--raw <file>]
//!
//! The shape (keys, types, array lengths, `__typename`s) contains no personal data. `--raw` writes the full answer to a
//! file for building anonymised fixtures; never commit raw answers.

use librespot_core::{cache::Cache, config::SessionConfig, session::Session};
use serde_json::Value;
use singularityd::catalogue::Dirs;
use singularityd::queries::QueryRegistry;

fn shape(v: &Value, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                let label = match x {
                    Value::Object(_) => String::new(),
                    Value::Array(a) => format!("[{}]", a.len()),
                    Value::String(s) if k == "__typename" || k == "uri" && s.split(':').count() == 3 => {
                        // a typename, or a URI's kind (never its id)
                        let s = if k == "uri" { s.rsplitn(2, ':').nth(1).unwrap_or("") } else { s };
                        format!(": {s:?}")
                    }
                    Value::String(_) => ": str".into(),
                    Value::Number(_) => ": num".into(),
                    Value::Bool(_) => ": bool".into(),
                    Value::Null => ": null".into(),
                };
                out.push_str(&format!("{pad}{k}{label}\n"));
                match x {
                    Value::Object(_) => shape(x, depth + 1, out),
                    Value::Array(a) if a.first().is_some_and(|f| f.is_object()) => shape(&a[0], depth + 1, out),
                    _ => {}
                }
            }
        }
        _ => out.push_str(&format!("{pad}{v:?}\n")),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (op, vars) = (args.get(1).expect("operation"), args.get(2).map(String::as_str).unwrap_or("{}"));
    let dirs = Dirs::from_env();
    let cache = Cache::new(Some(&dirs.credentials), None, None, None)?;
    let creds = singularityd::catalogue::saved_credentials(&dirs).ok_or_else(|| anyhow::anyhow!("no saved login; log in from the Client first"))?;
    let session = Session::new(SessionConfig::default(), Some(cache));
    session.connect(creds, true).await?;
    let mut registry = QueryRegistry::new(dirs.queries);
    let hash = registry.hash(op).await?;
    let t = std::time::Instant::now();
    let answer = session.spclient().pathfinder_query(op, &hash, serde_json::from_str(vars)?).await?;
    eprintln!("{op}: {:?}", t.elapsed());
    if let Some(i) = args.iter().position(|a| a == "--raw") {
        std::fs::write(&args[i + 1], serde_json::to_vec_pretty(&answer)?)?;
    }
    let mut out = String::new();
    shape(&answer, 0, &mut out);
    print!("{out}");
    Ok(())
}

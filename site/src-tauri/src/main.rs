#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use futures_util::{stream, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::{
    path::{Component, Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tauri::{AppHandle, Emitter, Manager};
use tokio::fs;

const MANIFEST: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
const JAVA_ALL: &str = "https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";

type Res<T> = Result<T, String>;
fn e<T: ToString>(x: T) -> String {
    x.to_string()
}

// ---------- helpers ----------
async fn get_json(c: &reqwest::Client, url: &str) -> Res<Value> {
    c.get(url).send().await.map_err(e)?.error_for_status().map_err(e)?.json().await.map_err(e)
}

async fn sha1_of(p: &Path) -> Option<String> {
    let b = fs::read(p).await.ok()?;
    Some(hex::encode(Sha1::digest(&b)))
}

struct Job {
    url: String,
    dest: PathBuf,
    sha1: Option<String>,
}

async fn download(c: &reqwest::Client, j: &Job) -> Res<()> {
    if j.dest.exists() {
        match &j.sha1 {
            Some(h) => {
                if sha1_of(&j.dest).await.as_deref() == Some(h.as_str()) {
                    return Ok(());
                }
            }
            None => return Ok(()),
        }
    }
    if let Some(p) = j.dest.parent() {
        fs::create_dir_all(p).await.map_err(e)?;
    }
    let bytes = c.get(&j.url).send().await.map_err(e)?.error_for_status().map_err(e)?.bytes().await.map_err(e)?;
    if let Some(h) = &j.sha1 {
        if hex::encode(Sha1::digest(&bytes)) != *h {
            return Err(format!("sha1 mismatch: {}", j.url));
        }
    }
    fs::write(&j.dest, &bytes).await.map_err(e)
}

async fn run_jobs(app: &AppHandle, c: &reqwest::Client, label: &str, jobs: Vec<Job>) -> Res<()> {
    let total = jobs.len();
    let done = Arc::new(AtomicUsize::new(0));
    let _ = app.emit("progress", json!({ "label": label, "done": 0, "total": total }));
    let results: Vec<Res<()>> = stream::iter(jobs)
        .map(|j| {
            let (c, app, done, label) = (c.clone(), app.clone(), done.clone(), label.to_string());
            async move {
                let r = download(&c, &j).await;
                let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                let _ = app.emit("progress", json!({ "label": label, "done": n, "total": total }));
                r
            }
        })
        .buffer_unordered(16)
        .collect()
        .await;
    results.into_iter().collect()
}

fn safe_join(base: &Path, rel: &str) -> Res<PathBuf> {
    let p = Path::new(rel);
    if p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir | Component::RootDir | Component::Prefix(_))) {
        return Err(format!("unsafe path: {rel}"));
    }
    Ok(base.join(p))
}

/// Mojang "rules" — Windows x64 only; entries gated by launcher "features" are skipped.
fn allowed(rules: Option<&Value>) -> bool {
    let Some(rs) = rules.and_then(|r| r.as_array()) else { return true };
    let mut ok = false;
    for r in rs {
        let matched = if r.get("features").is_some() {
            false
        } else {
            match r.get("os") {
                None => true,
                Some(o) => o.get("name").map_or(true, |n| n == "windows") && o.get("arch").map_or(true, |a| a != "x86"),
            }
        };
        if matched {
            ok = r["action"] == "allow";
        }
    }
    ok
}

fn expand(s: &str, vars: &[(&str, String)]) -> String {
    let mut o = s.to_string();
    for (k, v) in vars {
        o = o.replace(&format!("${{{k}}}"), v);
    }
    o
}

fn collect_args(list: Option<&Value>, vars: &[(&str, String)]) -> Vec<String> {
    let mut out = vec![];
    if let Some(a) = list.and_then(|l| l.as_array()) {
        for x in a {
            match x {
                Value::String(s) => out.push(expand(s, vars)),
                Value::Object(_) if allowed(x.get("rules")) => match &x["value"] {
                    Value::String(s) => out.push(expand(s, vars)),
                    Value::Array(vs) => {
                        for s in vs.iter().filter_map(|v| v.as_str()) {
                            out.push(expand(s, vars));
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
    out
}

async fn ensure_java(app: &AppHandle, c: &reqwest::Client, root: &Path, comp: &str) -> Res<PathBuf> {
    let all = get_json(c, JAVA_ALL).await?;
    let entry = all["windows-x64"][comp].get(0).ok_or("no java runtime for this version")?;
    let man = get_json(c, entry["manifest"]["url"].as_str().ok_or("bad java manifest")?).await?;
    let base = root.join("runtime").join(comp);
    let mut jobs = vec![];
    for (p, f) in man["files"].as_object().ok_or("bad java files")? {
        if f["type"] == "file" {
            let raw = &f["downloads"]["raw"];
            jobs.push(Job {
                url: raw["url"].as_str().ok_or("bad url")?.to_string(),
                dest: safe_join(&base, p)?,
                sha1: raw["sha1"].as_str().map(String::from),
            });
        }
    }
    run_jobs(app, c, "Java runtime", jobs).await?;
    Ok(base.join("bin").join("javaw.exe"))
}

// ---------- commands ----------
#[tauri::command]
async fn list_versions() -> Res<Vec<String>> {
    let m = get_json(&reqwest::Client::new(), MANIFEST).await?;
    Ok(m["versions"]
        .as_array()
        .ok_or("bad manifest")?
        .iter()
        .filter(|v| v["type"] == "release")
        .filter_map(|v| v["id"].as_str().map(String::from))
        .take(40)
        .collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LaunchReq {
    version: String,
    username: String,
    ram_mb: Option<u32>,
}

/// ตรวจ/โหลดไฟล์เกมที่ขาดหรือเปลี่ยน (sha1) ทุกครั้งก่อนเล่น = auto-update ไฟล์เกมในตัว
#[tauri::command]
async fn launch(app: AppHandle, req: LaunchReq) -> Res<()> {
    let root = app.path().app_data_dir().map_err(e)?.join("minecraft");
    let game_dir = root.join("game");
    fs::create_dir_all(&game_dir).await.map_err(e)?;
    let c = reqwest::Client::new();

    // 1) version json
    let _ = app.emit("progress", json!({ "label": "Version info", "done": 0, "total": 1 }));
    let m = get_json(&c, MANIFEST).await?;
    let url = m["versions"]
        .as_array()
        .and_then(|a| a.iter().find(|v| v["id"] == req.version.as_str()))
        .and_then(|v| v["url"].as_str())
        .ok_or("version not found")?
        .to_string();
    let v = get_json(&c, &url).await?;
    let vdir = root.join("versions").join(&req.version);
    fs::create_dir_all(&vdir).await.map_err(e)?;
    fs::write(vdir.join(format!("{}.json", req.version)), v.to_string()).await.map_err(e)?;

    // 2) client + libraries
    let client_jar = vdir.join(format!("{}.jar", req.version));
    let mut jobs = vec![Job {
        url: v["downloads"]["client"]["url"].as_str().ok_or("no client")?.to_string(),
        dest: client_jar.clone(),
        sha1: v["downloads"]["client"]["sha1"].as_str().map(String::from),
    }];
    let mut cp: Vec<String> = vec![];
    for lib in v["libraries"].as_array().ok_or("no libraries")? {
        // รองรับ 1.19+ (natives อยู่ในชื่อ library); ของเก่าที่มี "natives" จะข้าม
        if !allowed(lib.get("rules")) || lib.get("natives").is_some() {
            continue;
        }
        let Some(a) = lib["downloads"].get("artifact") else { continue };
        let dest = safe_join(&root.join("libraries"), a["path"].as_str().ok_or("bad lib")?)?;
        cp.push(dest.to_string_lossy().to_string());
        jobs.push(Job { url: a["url"].as_str().ok_or("bad lib url")?.to_string(), dest, sha1: a["sha1"].as_str().map(String::from) });
    }
    run_jobs(&app, &c, "Libraries", jobs).await?;
    cp.push(client_jar.to_string_lossy().to_string());

    // 3) assets
    let ai = &v["assetIndex"];
    let ai_id = ai["id"].as_str().ok_or("no assetIndex")?.to_string();
    let idx_path = root.join("assets").join("indexes").join(format!("{ai_id}.json"));
    run_jobs(&app, &c, "Asset index", vec![Job {
        url: ai["url"].as_str().ok_or("no asset url")?.to_string(),
        dest: idx_path.clone(),
        sha1: ai["sha1"].as_str().map(String::from),
    }]).await?;
    let idx: Value = serde_json::from_slice(&fs::read(&idx_path).await.map_err(e)?).map_err(e)?;
    let mut ajobs = vec![];
    for (_, o) in idx["objects"].as_object().ok_or("bad asset index")? {
        let h = o["hash"].as_str().ok_or("bad hash")?;
        ajobs.push(Job {
            url: format!("https://resources.download.minecraft.net/{}/{}", &h[..2], h),
            dest: root.join("assets").join("objects").join(&h[..2]).join(h),
            sha1: Some(h.to_string()),
        });
    }
    run_jobs(&app, &c, "Assets", ajobs).await?;

    // 4) Java (ไม่ต้องให้ผู้เล่นลง Java เอง)
    let comp = v["javaVersion"]["component"].as_str().unwrap_or("jre-legacy");
    let java = ensure_java(&app, &c, &root, comp).await?;

    // 5) args + start
    let natives = root.join("natives");
    fs::create_dir_all(&natives).await.map_err(e)?;
    let uuid: String = hex::encode(Sha1::digest(format!("OfflinePlayer:{}", req.username))).chars().take(32).collect();
    let vars: Vec<(&str, String)> = vec![
        ("natives_directory", natives.to_string_lossy().into()),
        ("launcher_name", "NovaLauncher".into()),
        ("launcher_version", env!("CARGO_PKG_VERSION").into()),
        ("classpath", cp.join(";")),
        ("classpath_separator", ";".into()),
        ("library_directory", root.join("libraries").to_string_lossy().into()),
        ("auth_player_name", req.username.clone()),
        ("auth_uuid", uuid),
        ("auth_access_token", "0".into()), // TODO: ใส่ token จาก Microsoft login
        ("auth_xuid", "0".into()),
        ("clientid", "0".into()),
        ("user_type", "msa".into()),
        ("version_name", req.version.clone()),
        ("version_type", "NovaLauncher".into()),
        ("game_directory", game_dir.to_string_lossy().into()),
        ("assets_root", root.join("assets").to_string_lossy().into()),
        ("assets_index_name", ai_id),
    ];
    let mut args = vec![format!("-Xmx{}M", req.ram_mb.unwrap_or(3072).clamp(1024, 32768))];
    args.extend(collect_args(v["arguments"].get("jvm"), &vars));
    args.push(v["mainClass"].as_str().ok_or("no mainClass")?.to_string());
    args.extend(collect_args(v["arguments"].get("game"), &vars));

    std::process::Command::new(java).args(args).current_dir(&game_dir).spawn().map_err(e)?;
    let _ = app.emit("launched", ());
    Ok(())
}

/// Sync modpack/config จากเว็บ: {"version":"1.0","files":[{"path","url","sha1"}],"delete":["mods/old.jar"]}
#[tauri::command]
async fn sync_pack(app: AppHandle, url: String) -> Res<String> {
    let game_dir = app.path().app_data_dir().map_err(e)?.join("minecraft").join("game");
    let c = reqwest::Client::new();
    let m = get_json(&c, &url).await?;
    let mut jobs = vec![];
    for f in m["files"].as_array().ok_or("bad pack")? {
        jobs.push(Job {
            url: f["url"].as_str().ok_or("no url")?.to_string(),
            dest: safe_join(&game_dir, f["path"].as_str().ok_or("no path")?)?,
            sha1: Some(f["sha1"].as_str().ok_or("sha1 required")?.to_string()), // บังคับ sha1 กันไฟล์ถูกสลับ
        });
    }
    run_jobs(&app, &c, "Modpack", jobs).await?;
    for d in m["delete"].as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
        let _ = fs::remove_file(safe_join(&game_dir, d)?).await;
    }
    Ok(m["version"].as_str().unwrap_or("?").to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![list_versions, launch, sync_pack])
        .run(tauri::generate_context!())
        .expect("error while running launcher");
}

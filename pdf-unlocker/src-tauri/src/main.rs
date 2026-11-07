#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// PDF 权限解锁工具
// 原理：调用内置 qpdf 引擎，用空用户密码解密后重写 PDF，
// 去除整层加密字典，从而放开被所有者密码锁住的权限（编辑/复制/打印等）。
// 不处理真正的打开密码（无密码则提示用户输入）。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(Serialize, Clone)]
struct PdfInfo {
    path: String,
    name: String,
    size: u64,
    pages: u32,
    encrypted: bool,
    needs_password: bool,
    algo: String,
    denied: Vec<String>,
}

#[derive(Serialize)]
struct UnlockResult {
    out_path: String,
    size: u64,
}

/// 定位 qpdf 引擎：优先主程序旁边的 qpdf/bin/qpdf.exe（绿色版布局），
/// 其次 tauri 资源目录（安装版布局）。
fn qpdf_exe() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            let candidates = [
                d.join("qpdf/bin/qpdf.exe"),
                d.join("resources/qpdf/bin/qpdf.exe"),
            ];
            for c in candidates {
                if c.exists() {
                    return Some(c);
                }
            }
        }
    }
    None
}

/// 运行 qpdf，返回 (退出码, 合并的 stdout+stderr 文本)
fn run_qpdf(args: &[&str]) -> Result<(i32, String), String> {
    let qpdf = qpdf_exe().ok_or(
        "未找到 qpdf 引擎，请确认 qpdf/bin/qpdf.exe 与主程序在同一目录",
    )?;
    let mut cmd = Command::new(&qpdf);
    cmd.args(args);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW，不闪黑框
    let out = cmd
        .output()
        .map_err(|e| format!("无法启动 qpdf：{}", e))?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.code().unwrap_or(-1), text))
}

fn human_error(raw: &str) -> String {
    // 取 qpdf 报错的最后一行，转成友好提示
    let line = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .next_back()
        .unwrap_or(raw);
    if line.to_lowercase().contains("password") {
        "需要打开密码：这个 PDF 设置了打开密码，请输入密码后重试".into()
    } else if line.to_lowercase().contains("not a valid pdf")
        || line.to_lowercase().contains("cannot be parsed")
        || line.to_lowercase().contains("unrecognized")
    {
        "不是有效的 PDF 文件".into()
    } else {
        format!("qpdf 报错：{}", line)
    }
}

fn denied_labels(v: &serde_json::Value) -> Vec<String> {
    const MAP: &[(&str, &str)] = &[
        ("modify", "修改内容（编辑）"),
        ("modifyother", "修改其他"),
        ("modifyannotations", "修改注释"),
        ("modifyassembly", "拼合/组织文档"),
        ("modifyforms", "修改表单"),
        ("extract", "复制/提取内容"),
        ("accessibility", "辅助功能提取"),
        ("printlow", "打印（低分辨率）"),
        ("printhigh", "打印（高分辨率）"),
    ];
    let caps = &v["encrypt"]["capabilities"];
    let mut out = Vec::new();
    for (key, label) in MAP {
        if caps.get(key).and_then(|x| x.as_bool()) == Some(false) {
            out.push(label.to_string());
        }
    }
    out
}

fn algo_label(v: &serde_json::Value) -> String {
    let p = &v["encrypt"]["parameters"];
    let bits = p["bits"].as_i64().unwrap_or(0);
    let r = p["R"].as_i64().unwrap_or(0);
    let method = p["filemethod"].as_str().unwrap_or("");
    let m = match method {
        "AESv3" => "AES-256",
        "AESv2" => "AES-128",
        "RC4" => "RC4",
        other => other,
    };
    format!("{}（{}-bit，R{}）", m, bits, r)
}

fn parse_pages(path: &str) -> u32 {
    if let Ok((code, text)) = run_qpdf(&["--show-npages", path]) {
        if code == 0 {
            if let Some(n) = text
                .lines()
                .find(|l| !l.trim().is_empty())
                .and_then(|l| l.trim().parse::<u32>().ok())
            {
                return n;
            }
        }
    }
    0
}

fn inspect_inner(path: &str, password: Option<&str>) -> Result<PdfInfo, String> {
    let p = Path::new(path);
    if !p.exists() {
        return Err("文件不存在".into());
    }
    let meta = fs::metadata(p).map_err(|e| format!("无法读取文件信息：{}", e))?;
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());

    let mut args: Vec<String> = vec!["--json=2".into(), "--json-key=encrypt".into()];
    if let Some(pw) = password {
        args.push(format!("--password={}", pw));
    }
    args.push(path.into());
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let (code, text) = run_qpdf(&arg_refs)?;
    if code != 0 {
        if text.to_lowercase().contains("password") {
            return Ok(PdfInfo {
                path: path.into(),
                name,
                size: meta.len(),
                pages: 0,
                encrypted: true,
                needs_password: true,
                algo: String::new(),
                denied: vec![],
            });
        }
        return Err(human_error(&text));
    }

    // stdout 是纯 JSON（stderr 可能有无关警告，JSON 从首个 { 开始解析）
    let json_start = text.find('{').ok_or_else(|| human_error(&text))?;
    let v: serde_json::Value =
        serde_json::from_str(&text[json_start..]).map_err(|e| format!("解析失败：{}", e))?;

    let encrypted = v
        .pointer("/encrypt/encrypted")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);

    Ok(PdfInfo {
        path: path.into(),
        name,
        size: meta.len(),
        pages: parse_pages(path),
        encrypted,
        needs_password: false,
        algo: if encrypted { algo_label(&v) } else { String::new() },
        denied: if encrypted { denied_labels(&v) } else { vec![] },
    })
}

#[tauri::command]
fn pick_pdfs() -> Result<Vec<PdfInfo>, String> {
    let files = rfd::FileDialog::new()
        .set_title("选择要解锁的 PDF 文件")
        .add_filter("PDF 文件", &["pdf"])
        .pick_files()
        .unwrap_or_default();
    let mut out = Vec::new();
    for f in files {
        match inspect_inner(&f.to_string_lossy(), None) {
            Ok(info) => out.push(info),
            Err(e) => {
                if !out.is_empty() || out.is_empty() {
                    // 单个文件失败不阻塞其他文件，记录为一条错误信息
                    eprintln!("inspect failed: {} - {}", f.display(), e);
                }
            }
        }
    }
    Ok(out)
}

#[tauri::command]
fn inspect_pdf(path: String, password: Option<String>) -> Result<PdfInfo, String> {
    inspect_inner(&path, password.as_deref())
}

#[tauri::command]
fn unlock_pdf(path: String, password: Option<String>) -> Result<UnlockResult, String> {
    let info = inspect_inner(&path, password.as_deref())?;
    if info.needs_password {
        return Err("需要打开密码，请先输入密码".into());
    }
    if !info.encrypted {
        return Err("该文件未加密，无需解锁".into());
    }

    // 输出路径：同目录 <原名>_已解锁.pdf；重名则追加 (1)、(2)…
    let src = Path::new(&path);
    let dir = src.parent().ok_or("无法确定输出目录")?;
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "output".into());
    let mut dst = dir.join(format!("{}_已解锁.pdf", stem));
    let mut n = 1;
    while dst.exists() {
        dst = dir.join(format!("{}_已解锁({}).pdf", stem, n));
        n += 1;
    }
    let dst_str = dst.to_string_lossy().into_owned();

    let mut args: Vec<String> = vec!["--decrypt".into()];
    if let Some(pw) = password {
        args.push(format!("--password={}", pw));
    }
    args.push(path.clone());
    args.push(dst_str.clone());
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    let (code, text) = run_qpdf(&arg_refs)?;
    if code != 0 {
        return Err(human_error(&text));
    }

    // 校验输出文件确实无加密
    match run_qpdf(&["--show-encryption", &dst_str]) {
        Ok((_, t)) if t.contains("File is not encrypted") => {}
        _ => return Err("解锁后校验失败，请重试".into()),
    }

    let size = fs::metadata(&dst).map(|m| m.len()).unwrap_or(0);
    Ok(UnlockResult {
        out_path: dst_str,
        size,
    })
}

#[tauri::command]
fn open_in_explorer(path: String) -> Result<(), String> {
    let win_path = path.replace('/', "\\");
    #[cfg(windows)]
    {
        Command::new("explorer")
            .arg("/select,")
            .arg(&win_path)
            .spawn()
            .map_err(|e| format!("打开文件夹失败：{}", e))?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err("仅支持 Windows".into())
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            pick_pdfs,
            inspect_pdf,
            unlock_pdf,
            open_in_explorer
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

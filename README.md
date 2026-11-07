# tools

个人工具集。

## PDF 解锁工具（pdf-unlocker）

去除 PDF 的「权限限制锁」（WPS / Adobe 加的编辑、复制、打印限制），保留打开密码不动。
基于 Tauri v2 + qpdf 引擎，免安装绿色版。

### 下载

到 [Releases](https://github.com/hy89/tools/releases) 页面下载 `PDF解锁工具_绿色版.zip`，
解压后双击 `PDF解锁工具.exe` 即可，无需安装。

### 功能

- 拖拽 / 选择 PDF（支持批量），自动识别加密类型与被禁权限
- 一键解锁 / 全部解锁，输出 `原名_已解锁.pdf`，原文件不动
- 带打开密码的文件可输入密码后解锁（不做密码破解）
- 解锁后自动校验输出文件确无加密

### 本地构建

```bash
cd pdf-unlocker/src-tauri
cargo build --release
```

构建产物在 `target/release/pdf-unlocker.exe`，需与 `bin/qpdf/bin/` 目录（qpdf 引擎 + 运行库）
一起分发——绿色版目录结构：

```
PDF解锁工具/
├── PDF解锁工具.exe        # 主程序
└── qpdf/bin/              # qpdf 12.4.2 引擎（Apache-2.0）
    ├── qpdf.exe
    └── *.dll
```

### 发版

推送 tag 即可自动构建并发布 Release（由 GitHub Actions 完成）：

```bash
git tag v1.x.x
git push origin v1.x.x
```

### 目录说明

| 路径 | 内容 |
|---|---|
| `pdf-unlocker/ui/` | 前端界面（原生 HTML/JS，无 npm 依赖） |
| `pdf-unlocker/src-tauri/src/` | Rust 后端（Tauri commands） |
| `pdf-unlocker/src-tauri/bin/qpdf/` | qpdf 引擎二进制（随仓库分发，构建必需） |
| `pdf-unlocker/src-tauri/target/` | 构建缓存（已忽略，本地约 1.2 GB，可随时删除） |

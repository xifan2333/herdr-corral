# corral

VS Code 风格的终端工作台（Rust）。**开发中**。

**先是独立 TUI，再是可选的 Herdr 插件** — 和 gitui / herdr-file-viewer 同一思路。

## 两种启动方式

| 模式 | 启动 | 说明 |
|---|---|---|
| **standalone** | `cargo run --release` / `./target/release/corral` | 普通终端程序，不依赖 Herdr |
| **plugin** | `herdr plugin action invoke corral.open` | Herdr 开一个 split pane 跑同一二进制 |

Host 边界在 `corral::host`：有 `HERDR_ENV` / `HERDR_PLUGIN_*` → plugin，否则 → standalone。  
缺 Herdr config 时 theme 回落到内置 `terminal` 色表。

## 形状

```text
corral 进程
 ├── left  container   (后续挂 Explorer / SCM / GitHub)
 └── right container   (后续挂主视图)
```

不是两个宿主 pane；左右是进程内 region。

## 现状

| 层 | 状态 |
|---|---|
| `host` | 已实现：plugin / standalone 检测 + LaunchContext |
| `theme` | 已实现：Herdr 主题表；无 config → `terminal` |
| `layout` | 已实现：左右容器几何 + focus |
| `app` | 已实现：空容器骨架 |
| Explorer / SCM / GitHub | 未实现 |

## 开发

```bash
# 独立模式
cargo build --release
./target/release/corral

# 插件模式
herdr plugin link .
herdr plugin action invoke corral.open
```

快捷键：`Tab` / `h` `l` 切焦点，`q` / `Esc` 退出。

## 目录

```text
herdr-corral/
  herdr-plugin.toml      # 仅插件分发需要
  scripts/open-corral.sh
  src/
    main.rs
    lib.rs
    host.rs              # Herdr 边界（可选）
    theme.rs
    layout.rs
    app.rs
```

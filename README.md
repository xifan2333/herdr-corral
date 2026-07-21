# corral

VS Code 风格的 Herdr 终端工作台插件（Rust）。**开发中**。

形状对齐 [herdr-file-viewer](https://github.com/smarzban/herdr-file-viewer)：

- Herdr 只开 **一个** plugin pane
- 左 / 右容器是 **同一进程内** 的两个 region（ratatui 自己画），不是两个 Herdr pane
- 后续 Explorer / SCM / GitHub 都挂进这两个容器

## 现状

| 层 | 状态 |
|---|---|
| `corral::theme` | 已实现：读 Herdr config，解析当前主题色表（`ratatui::Color`） |
| `corral::layout` | 已实现：进程内左右容器几何 + focus |
| `corral::app` | 已实现：单 pane 宿主 TUI，画两个空容器骨架 |
| Explorer / SCM / GitHub | 未实现（下一步往容器里挂） |

## 开发

```bash
cargo build --release
herdr plugin link .
herdr plugin action invoke corral.open
```

宿主内快捷键：

- `Tab` / `h` `l`：切换左右容器焦点
- `q` / `Esc`：退出

## 目录

```text
herdr-corral/
  herdr-plugin.toml
  Cargo.toml
  scripts/
    open-corral.sh     # launch / focus / toggle-close
  src/
    lib.rs
    main.rs            # thin binary → corral::run()
    theme.rs           # Herdr 主题色表
    layout.rs          # 左右容器
    app.rs             # 宿主事件循环
  README.md
```

## 约定

- 回调 Herdr 优先用 `HERDR_BIN_PATH`
- 用户配置放 `HERDR_PLUGIN_CONFIG_DIR`，运行时状态放 `HERDR_PLUGIN_STATE_DIR`
- 不要把凭据/状态写进插件根目录

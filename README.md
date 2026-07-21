# corral

VS Code 风格的**左侧边栏**（Rust）。**开发中**。

对齐 [herdr-sidebar](https://github.com/alexarthurs/herdr-sidebar) 的形状，而不是整页 workbench：

```text
┌────────────┬──────────────────────────────┐
│  Sidebar   │  用户原有 panes / 未来 Preview │
│  [][][] │                              │
│  树 / 列表  │                              │
└────────────┴──────────────────────────────┘
```

- **一个** left-docked Herdr pane
- 顶栏横排 feature icons：Explorer / SCM / GitHub（进程内切换）
- 详情（文件 / diff / PR）→ **以后**用独立 preview pane + control file
- 也可 standalone：`./target/release/corral`

## 开发

```bash
cargo build --release

# 独立
./target/release/corral

# 插件（左 dock）
herdr plugin link .
herdr plugin action invoke corral.open
```

快捷键：`1`/`2`/`3` 或 `j`/`k`/`Tab` 切 feature，`q` 退出。

## 模块

| 模块 | 作用 |
|---|---|
| `host` | plugin / standalone 上下文 |
| `theme` | Herdr 主题色表 |
| `icons` | Nerd Font 检测 |
| `feature` | Explorer / SCM / GitHub |
| `layout` | activity + body 几何 |
| `app` | sidebar 事件循环 |

## 下一步

1. Explorer 文件树  
2. preview 子命令 + ctl 协议  
3. SCM / GitHub view  

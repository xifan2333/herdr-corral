# workbench.vscode

VS Code 风格的 Herdr workbench activity bar：

| 面板 | 工具 | Action |
|------|------|--------|
| **Explorer** | [`yazi`](https://yazi-rs.github.io/) | `workbench.vscode.toggle-explorer` |
| **Source Control** | [`lazygit`](https://github.com/jesseduffield/lazygit) | `workbench.vscode.toggle-scm` |
| **GitHub** | [`gh`](https://cli.github.com/) + `fzf` | `workbench.vscode.toggle-github` |

每个 toggle 都是 **open → focus → close** 循环（当前 tab 内）。  
GitHub 面板另有 Issues / PRs / Actions 快捷入口，以及 Ctrl-click issue/PR 链接预览。

## 依赖

```bash
# Arch 示例
sudo pacman -S yazi lazygit github-cli fzf
gh auth login
```

## Link / 使用

```bash
herdr plugin link /path/to/herdr-workbench/plugins/workbench

herdr plugin action invoke workbench.vscode.toggle-explorer
herdr plugin action invoke workbench.vscode.toggle-scm
herdr plugin action invoke workbench.vscode.toggle-github

herdr plugin action invoke workbench.vscode.github-issues
herdr plugin action invoke workbench.vscode.github-prs
herdr plugin action invoke workbench.vscode.github-actions
```

## 推荐快捷键

写进 `~/.config/herdr/config.toml`：

```toml
[[keys.command]]
key = "prefix+e"
type = "plugin_action"
command = "workbench.vscode.toggle-explorer"
description = "toggle explorer"

[[keys.command]]
key = "prefix+g"
type = "plugin_action"
command = "workbench.vscode.toggle-scm"
description = "toggle source control"

[[keys.command]]
key = "prefix+shift+g"
type = "plugin_action"
command = "workbench.vscode.toggle-github"
description = "toggle github hub"
```

改完后：`herdr server reload-config`。

## 布局

- 面板默认 **右侧 split**（Herdr split 仅支持 `right|down`）
- 当前 workspace cwd / focused pane cwd 会传给 pane
- Ctrl-click 匹配的 GitHub issue/PR URL → 右侧/左侧 split 预览详情

## 目录

```text
workbench/
  herdr-plugin.toml
  README.md
  scripts/
    lib.sh                 # toggle / cwd / gh repo helpers
    toggle-pane.sh         # generic open/focus/close
    run-explorer.sh
    run-scm.sh
    run-github.sh          # interactive Issues/PRs/Actions hub
    open-github-section.sh
    open-link-preview.sh
    run-link-preview.sh
```

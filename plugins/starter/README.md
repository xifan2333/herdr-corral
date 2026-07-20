# workbench.starter

最小 Herdr 插件骨架，演示：

- manifest 声明 actions
- 通过 `HERDR_BIN_PATH` 回调 Herdr CLI
- 打印运行时注入的 context / config / state 路径

## Link

```bash
herdr plugin link /path/to/herdr-workbench/plugins/starter
herdr plugin action invoke workbench.starter.hello
herdr plugin action invoke workbench.starter.list-workspaces
herdr plugin action invoke workbench.starter.dump-context
```

## Actions

| Action id | 全局 id | 作用 |
|-----------|---------|------|
| `hello` | `workbench.starter.hello` | 打印基础 env |
| `list-workspaces` | `workbench.starter.list-workspaces` | 调用 `herdr workspace list` |
| `dump-context` | `workbench.starter.dump-context` | 转储完整 plugin context |

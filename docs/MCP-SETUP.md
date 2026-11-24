# 接入 PLAhelper MCP（pla-mcp）

`pla-mcp.exe` 是一个 stdio MCP 服务器：客户端启动它，经标准输入 / 输出收发 JSON-RPC。
工具清单与工作方式见 `docs/know-how/mcp-tools.md`；给 agent 的使用守则写在服务器的 `instructions` 里，客户端连上就会拿到。

## 前提

- 已编译：`<repo>\src\target\release\pla-mcp.exe`（`cargo build --release` 于 `src`）。
- Plasticity 以 CDP 模式运行（建模类 `model_*` 工具要用）；出图 / BOM 类工具还要 pla-ui 在跑。
- 不需要任何环境变量或参数。

## 一律写绝对路径

仓库根的 `.mcp.json` 用的是相对路径 `src/target/release/pla-mcp.exe`，只有在仓库根目录启动 Claude Code 时才找得到。
在别处使用，或者用其他客户端时，都写 **exe 的绝对路径**。下面以本机路径为例，换成你自己的 `<repo>`：

```
C:\Users\frg\Documents\BaiduSyncdisk\AIAgent\PLAHelper\repo\src\target\release\pla-mcp.exe
```

JSON 里的反斜杠要写成 `\\`（或者直接用 `/`）。

### Claude Code

全局登记（任何目录都能用），在 PowerShell 里：

```powershell
claude mcp add --scope user plahelper -- "C:\Users\frg\Documents\BaiduSyncdisk\AIAgent\PLAHelper\repo\src\target\release\pla-mcp.exe"
```

或者在某个项目根目录放 `.mcp.json`：

```json
{
  "mcpServers": {
    "plahelper": {
      "command": "C:\\Users\\frg\\Documents\\BaiduSyncdisk\\AIAgent\\PLAHelper\\repo\\src\\target\\release\\pla-mcp.exe",
      "args": []
    }
  }
}
```

项目级 `.mcp.json` 第一次使用时要在 Claude Code 里批准。

### WorkBuddy

编辑 `%USERPROFILE%\.workbuddy\mcp.json`：

```json
{
  "mcpServers": {
    "plahelper": {
      "command": "C:\\Users\\frg\\Documents\\BaiduSyncdisk\\AIAgent\\PLAHelper\\repo\\src\\target\\release\\pla-mcp.exe",
      "args": [],
      "env": {}
    }
  }
}
```

保存后到 WorkBuddy 的连接器管理页点「信任」才会激活。

### 其他 stdio 客户端（Cursor、Cline、Claude Desktop 等）

大多数客户端都是同一种 `mcpServers` 结构：

```json
{
  "mcpServers": {
    "plahelper": {
      "type": "stdio",
      "command": "C:/Users/frg/Documents/BaiduSyncdisk/AIAgent/PLAHelper/repo/src/target/release/pla-mcp.exe",
      "args": []
    }
  }
}
```

协议细节：每行一条 JSON-RPC，UTF-8，支持协议版本 2025-06-18 / 2025-03-26 / 2024-11-05。
不认识 `type` 字段的客户端去掉它即可。

## 更新之后

重新编译后，exe 可能被正在运行的客户端占用：

- 已连着的客户端要**重启 MCP 服务器**（或重启客户端）才会用上新版本；
- 编译时报 exe 被锁，就把旧文件改名为 `pla-mcp.old-<时间>.exe` 再编译，不要强行结束进程。

## 自检

客户端里调用 `model_transaction {action: "state"}`（只读）：返回撤销深度就说明已连上 Plasticity。
`status` 能返回说明 pla-ui 也在线。

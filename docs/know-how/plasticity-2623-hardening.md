# Plasticity 26.2.x 加固机制全解析（2026-10-29 实测，CDP 补丁为何失效）

本文记录 PLAhelper 在 26.2.3 上「为什么注入不进去」，以及**我们打通到哪一步、卡在哪一步**。
目的：下次再升级时不必重新趟一遍坑。所有结论均有实测证据，不是推测。

## 1. 目录结构变化（26.1.3 → 26.2.3）

| | 26.1.3（v6 补丁可打） | 26.2.3（不可打） |
|---|---|---|
| 资源 | `resources/app/` **明文目录** | `resources/app.asar`（108 MB，369 条目） |
| 主进程入口 | `app/.webpack/main/index.js` **明文 5.8 KB**（bytenode 加载器） | `app.asar/.webpack/main/index.js` **只剩 14 字节** `'use strict';` |
| 主包 | `app/.webpack/main/index.compiled.jsc`（标准 bytenode，`\xde\xc0` 签名） | `resources/plasticity.jsc.pack`（2.4 MB，**自定义容器 `PLSEC001`**） |
| Renderer | `app_window/index.js` 明文 bootstrap + `.jsc` | 同构，但 `.jsc` 也包了 `PLSEC001` |
| 容器 magic | bytenode：前 2 字节非 0 + `[0xDE,0xC0]` | `PLSEC001` + `u32 totalSize`（= 文件长度）+ 载荷 |

`plasticity.jsc.pack` 头部实锤：

```
50 4c 53 45 43 30 30 31  01 00 8c 00  0c ba 25 00  2a 00 0b 00 ...
'P  L  S  E  C  0  0  1'               ^ 0x0025ba0c == 文件长度 2,472,460
```

Renderer 的 `app_window/index.js`（602 B 明文）只是一个 `.jsc` 加载器：

```js
v8.setFlagsFromString('--no-lazy');
v8.setFlagsFromString('--no-flush-bytecode');
Module._extensions['.jsc'] = (module, filename) => {
  const wrapper = new vm.Script('', { cachedData: fs.readFileSync(filename), filename }).runInThisContext();
  wrapper.call(module.exports, module.exports, Module.createRequire(filename), module, filename, path.dirname(filename), process, global);
};
require("../app_window.compiled/index.jsc");
```

它能直接把 `PLSEC001` 喂给 V8 而不报错，说明**读文件的那层（Electron 的 asar 层 / exe）会把容器还原**。

## 2. exe 是定制版：`plasticity.jsc.pack` 由它加载

`Plasticity.exe` 内 UTF-16LE 字符串（一次性相邻出现）：

```
plasticity.jsc.pack   app.asar   .webpack   main   index.js
```

⇒ 他们改了 Electron 的 app 入口解析：**优先加载 resources 下的 jsc.pack**。
实测把 `plasticity.jsc.pack` 挪走后，app **完全起不来**（无 main.log、无对话框，直接退出），
说明**不会回退到 asar 里的 `index.js`**——主注入点 `.webpack/main/index.js` 是死路。

## 3. Electron fuses：注入类开关全关

在 exe 偏移 `209864208` 处找到 sentinel `dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX`，其后是
`u8 version=1`、`u8 len=9`、字符串 wire **`000011011`**（`'1'` = ENABLE）。

实测佐证：

- `ELECTRON_RUN_AS_NODE=1 Plasticity.exe script.js` → **照常启动 GUI**，`script.js` 被无视（RunAsNode 关闭）。
- 启动时日志：`NODE_OPTIONS ignored due to disabled nodeOptions fuse`（nodeOptions 关闭）。
- ⇒ `ELECTRON_RUN_AS_NODE` / `NODE_OPTIONS=--require` / `--inspect` / `--require` **全部不可用**。
- `remote-debugging-port` 也不能从命令行塞进去：亲眼见 main 拷进来就是两条
  `removeSwitch('remote-debugging-port'/'remote-debugging-pipe')`，electron Fossil 里也有这两个字符串打 ReserveAttrs。

## 4. ★ 我打通的部分：Electron 级 asar 完整性

期望值写在 **exe** 里（不是 asar 里！asar header 根节点只有 `files`），PE 资源类型为 `INTEGRITY` / `ELECTRONASAR`：

```
[{"file":"resources\\app.asar","alg":"SHA256","value":"1ceb53d0ef805fb516e0fb247dbcebed0e255860b4621877f8056b2012f46adb"}]PADDINGXPADDINGX...
```

（后面跟 `PADDINGX` 循环填充 —— 这本来就是留给就地改写的。）

**算法 = `sha256(header JSON 原始字节)`**（`@electron/asar` 写入的那段，即文件偏移 16 起、长度 L 的 JSON）。
验证方式：对原始 app.asar 做前缀滚动 sha256 比对。（**不是**整个文件、**不是** per-file。）

改法（长度恒定的十六进制串，就地覆写即可，无需重签）：

```python
import struct, hashlib, json
p = "resources/app.asar"
head = open(p,'rb').read(16)
L = struct.unpack('<IIII', head)[3]
json_bytes = open(p,'rb').read()[16:16+L]
new_hash = hashlib.sha256(json_bytes).hexdigest()
# 然后在 exe 里把 '"value":"' 之后的 64 个字符换成 new_hash
```

触发失败时的报错长这样（有用，贴在这里便于检索）：

```
[FATAL:electron\shell\common\asar\asar_util.cc:187] Integrity check failed for asar archive entry '<header>'
(expected vs actual, 101323 bytes)
```

**注意 Electron 只校验 `<header>` 这一段**，per-file `integrity` 是摆设（但我们重打包时仍会重算，保持一致）。

工具：`repo/tools/pla-cdp/pla-asar.js`（零依赖 asar 读写库，支持 `info/list/extract/replace/remove` + API）。
已验证：内容零改动地重打包 + 覆写该值 → **app 完整启动到 Main window loaded**。

## 5. ✗ 卡住的部分：Plasticity 自己的内容校验

在不碰任何原有文件内容的前提下（只重排 header key 顺序）时间是能跑的；**任何文件内容改动都会让 app 自杀**：

| 实验 | 结果 |
|---|---|
| 只重打包（内容零改动） | ✅ 正常启动到 Main window loaded |
| 替换 renderer `app_window/index.js`（加 marker，逻辑等价） | ❌ 退出 |
| 保留该文件原 per-file hash、只换内容 | ❌ 退出 |
| 改 `index.html`（插入 `<script>`）+ 新增一个文件 | ❌ 退出 |
| 把 `plasticity.jsc.pack` 挪走 | ❌ 完全起不来（无回退） |

失败形态高度一致：**`MAIN: App ready` 之后约 30 ms 就 `MAIN: Main window closed`，进程退出，
renderer 进程从未创建（无 `RENDER:` 日志、`ELECTRON_ENABLE_LOGGING` 也没有额外输出、无 crashpad dump）**。

⇒ 期望值是**按真实文件内容**比对的（保留 header 里的旧 hash 也救不了），而期望清单藏在加密主包
`plasticity.jsc.pack` 里 —— **改不了**。

**结论：26.2.3 的主进程与渲染进程都无法注入。** 唯一出路是逆向他们的校验（大工程，且每次自动升级失效）。

## 6. 顺带否掉的其他路线

| 路线 | 结论 |
|---|---|
| `--remote-debugging-port=9333` 命令行 | main 里 `removeSwitch` 把两个调试开关都摘了 |
| `--inspect` / `NODE_OPTIONS` / `ELECTRON_RUN_AS_NODE` | fuse 关闭（见 §3） |
| `app.asar.unpacked` 覆盖 | 只有 dll/.tx/.node，无 JS；且同样在完整性覆盖下 |
| 官方 bridge ws:8980 | 单向、只读 mesh，不能写选择/材质/命令（见 `research/2026-10-28-feasibility-312-33.md`）。且实测 26.2.3 未监听 8980 |
| Plasticity 自带调试开关 | 无。exe 里搜不到任何 `PLA_/PLASTICITY_/SKIP/DISABLE` 类环境变量名 |

## 7. 26.1.3 现状（重要）

`app-26.1.3` 目录还在、v6 补丁还在 `index.js` 里，但**现在启动即卡死**：
单进程停在 36 MB，内存不涨、没有任何网络连接、main.log 一个字不写、9333/8980 都不监听。
把 index.js 换成 2026-04 的**原始备份**也一样卡死 ⇒ 不是我们的补丁问题。
怀疑与 2026-10-26 被 26.2.3 重写过的 `~/.plasticity/license.lic` 有关（未敢动用户 license 文件验证）。

## 8. 复用清单

- `tools/pla-cdp/pla-asar.js`：asar 读写（重算 per-file integrity、原子落盘、保留 unpacked）。
- `tools/pla-cdp/pla-cdp-main-boot.js`：设想中的 v7 main 入口 bootstrap（CDP + 交接给原主包）。
  **目前被 §5 的校验挡住**，留着备用。
- 若将来某版本只开了 Electron 完整性而没有自家校验，按 §4 改 sha256 即可复活整套 CDP 能力。

## 9. ★★★ 突破（2026-10-29 06:40，推翻 §5/§6 的"无法注入"结论）

**单字节 fuse 补丁 → node inspector → editor 全链路打通，读写均验证。**

### 9.1 原理
- §5 说"主/渲染进程都注入不进去"是对 asar 内容改动而言；但 exe 的 **fuse wire 本身没有被 Plasticity 自家校验**（工程师已实测改 exe 内 integrity 值可正常启动 → 壳不校验 exe 自身）。
- fuse wire `000011011` 在 exe 文件偏移 **0xc824632**（sentinel `dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX` + ver1 + len9 之后）。
- Electron fuse 顺序：0=RunAsNode 1=CookieEncryption 2=NodeOptions 3=**NodeCliInspect** 4=EnableAsarIntegrity 5=OnlyLoadAppFromAsar ...
- **把 idx3 '0'→'1'（偏移 0xc824635）→ `--inspect` 复活**。这是 Node 级 inspector，不经过 main 里 removeSwitch('remote-debugging-port') 的 Chromium 开关 → 堵不住。
- 备份：`Plasticity.exe.pla-fuse-backup`（仅此一字节差异）。

### 9.2 完整链路（全部实测）
```
Plasticity.exe --inspect=127.0.0.1:9334     (fuse idx3=1)
  → node inspector (主进程, /json/list type=node)
  → Runtime.evaluate + process.mainModule.require('electron')
  → BrowserWindow.getAllWindows()[0].webContents.debugger.attach('1.3')   ← 内部 CDP, 无 removeSwitch
  → renderer 全套 CDP: Runtime/Page/HeapProfiler
```

### 9.3 找 editor（26.2.3 renderer 无全局 ed，全在闭包）
已证死路：全局扫描（`core`=lodash 污染, `__THREE__`=Three库）、require.cache（只有4个内建模块）、vm.Script wrapper（单包 webpack 闭包）、addEventListener handler 闭包链（[[Scopes]] 挖4层无命中）。
**活路 = 堆快照交叉定位**：
1. 同一 ws 会话内 `HeapProfiler.takeHeapSnapshot`（~62MB）收 chunk 落盘
2. 离线解析：node_fields=[type,name,id,self_size,edge_count,detachedness]（k=6），edge_fields=[type,name_or_index,to_node]（w=3，to_node=**元素偏移**）
3. 找**同时拥有 selection+shading+viewports 三条属性边**的源节点 → 唯一命中：name=**"Editor"**
4. **同会话** `HeapProfiler.getObjectByHeapObjectId(id)` 还原活体（快照 id 跨会话作废！）
5. `Runtime.callFunctionOn(oid, fn)` 读写

### 9.4 实测结果
- 读：`{sel:true, mode:Set, selSize:0, shading:true, vpCount:1}` ✅
- 写：`signals.selectionModeChanged.dispatch(selection.mode)` → ok ✅
- legacy：`window.__PLA_EDITOR__ = editor` 成功 → **老 PLAhelper JS 管线兼容**
- app 干净退出 ✅

### 9.5 产品化待办
- pla-cdp 工具：fuse 字节补丁（幂等+备份）+ `--inspect=9334` 启动器
- pla-bridge：连接层从 9333 CDP 改为 9334 inspector + attach + 堆快照找 editor（每次会话启动时跑一遍，~10s）
- editor 探活：renderer reload 后 oid 变化 → 需重跑快照定位（或挂在 Page.loadEventFired 后自动重定位）
- 26.1.3 老补丁路线整体作废（用户已拍板）

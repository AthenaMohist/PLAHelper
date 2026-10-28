# PLAhelper 便携包（给另一台电脑用）

包内文件：

| 文件 | 作用 |
| --- | --- |
| `pla-ui.exe` | PLAhelper 主程序（单文件，双击即用）。字体已内嵌，无外部依赖。 |
| `patch-cdp.bat` | 给 Plasticity 打 CDP 补丁的批处理（自动找安装位置、自动备份）。**仅适用于 26.2.1 及更早**。 |
| `pla-cdp-patch.js` | 补丁本体，`patch-cdp.bat` 要用它，两者必须放在同一目录。 |
| `pla-cdp-v7.js` | **新的检测/打补丁/还原工具**（推荐）。先 `status` 看清楚能不能打，再 `patch`。 |
| `pla-asar.js` | 零依赖 asar 读写库（`pla-cdp-v7.js` 与排查时用），`node pla-asar.js <asar> info`。 |
| `pla-cdp-main-boot.js` | v7 主入口引导脚本（26.2.x 被官方校验挡住，暂不可用，留档备用）。 |
| `README.md` | 本说明。 |

## 〇、先看这条：Plasticity 26.2.3 及以上**打不了补丁**

从 26.2.3 起 Plasticity 把主程序换成了加密包（`resources/plasticity.jsc.pack`，自定义容器
`PLSEC001`），并且会校验 `app.asar` 里每个文件的内容 —— 只要改了任何文件，程序就在
「App ready」之后几十毫秒自己退出（没有报错、界面都不出来）。所以 **26.2.3 上没有可用的
CDP 端口**，PLAhelper 依赖 CDP 的功能（选择集、视口状态、实体树）在这些版本上不可用。

判断方法（不用装任何东西）：

```
node pla-cdp-v7.js status
```

- `layout: legacy` + `patched: NO` → 可以打（见下节）。
- `layout: hardened` → **打不了**，工具会明确说出来并且**不会动任何文件**。

技术细节与证据：`repo/docs/know-how/plasticity-2623-hardening.md`。

## 一、使用步骤

1. **解压到任意目录**（不要放在需要管理员权限的目录里，比如 `C:\Program Files`，否则程序自己写不了 `hotkeys.json`）。
2. **先关掉 Plasticity**，然后**双击 `patch-cdp.bat`**：
   - 它会自动在常见位置找 Plasticity（`%LOCALAPPDATA%\Programs\Plasticity`、
     `C/D/E/F/G:\Program Files\Plasticity` 等），找到 `app-*\resources\app\.webpack\main\index.js`；
   - 没找到时，可以把那个 `index.js` **直接拖到 bat 上**；
   - 原文件会备份成 `index.js.pla-backup`（已存在就不覆盖）；
   - 已经打过补丁会直接提示并退出，不会重复打。
3. **重新启动 Plasticity**，然后双击 `pla-ui.exe`。
4. 验证补丁是否生效：浏览器打开 <http://127.0.0.1:9333/json/version> ，能看到 JSON 就说明 CDP 端口已开。
   Plasticity 的启动/异常日志在 `%USERPROFILE%\.pla-cdp-patch.log`。

## 二、关于权限（UAC）

- Plasticity 装在**用户目录**（`%LOCALAPPDATA%\Programs\Plasticity`）或者 **E:\Program Files 这类非系统盘**时，
  打补丁**不会弹任何权限窗口**。
- 只有在 `C:\Program Files\...` 这类受保护目录里，bat 才会先探测写权限、发现写不进去时**才**申请提权
  （这时会有一次 UAC 弹窗，属于 Windows 强制要求，无法绕过）。
- 想彻底不弹窗：在那台机器上把 Plasticity 装成**仅当前用户**安装（默认装在 `%LOCALAPPDATA%\Programs` 下）。

## 三、Windows SmartScreen / “已保护你的电脑”

从别的电脑拷过来的 exe/zip 会带 Mark-of-the-Web，Windows 可能提示“Windows 已保护你的电脑”：

- 右键 zip → **属性** → 勾选**解除锁定** → 应用，再解压；
- 已经解压的：右键 `pla-ui.exe` → 属性 → 解除锁定；
- 若仍提示：点“更多信息” → “仍要运行”。

（`patch-cdp.bat` 同理，首次运行可能也要解除锁定。）

## 四、数据存在哪

**全部跟随文档文件走，不跟 exe 走**（所以换机器不用搬数据，文档拷过去就带着）：

- 快照：`<文档目录>\snapshot\`
- 选择集：`<文档目录>\.pla\selections\<文档名>\`（SG001.json + 截图）
- 视口状态：`<文档目录>\.pla\viewstates\<文档名>\`（VS001.json + 截图）
- 只有快捷键配置 `hotkeys.json` 写在 exe 同级目录（要带走就一起拷）。

旧版本存在 exe 同级的 `selections-cdp/`、`viewstates/` 会在首次使用时**自动复制**到文档同级（旧文件保留不动）。

## 五、依赖条件

另一台机器上必须有：

1. **Plasticity**（且已用 `patch-cdp.bat` 打过 CDP 补丁）——否则界面一直显示“未绑定 / CDP 连接中…”，
   选择集、视口状态、实体树这些功能不可用；
2. Plasticity 自带的 bridge 服务在 `ws://127.0.0.1:8980`（正常启动即有；需要改地址时设环境变量 `PLA_BRIDGE_URL`）。

## 六、还原补丁

把 `index.js.pla-backup` 复制回去覆盖 `index.js` 即可，或者直接重装/更新 Plasticity。

# N5 资产(asset pack)放置: 实测(26.2.3, 2026-11-18, model1 + 用户的 screws.plasticitypack)

用户建议(2026-11-18): 螺丝这类常用标准件**优先用 Plasticity 的 asset 功能**, 不要每次现建; 放螺丝放在打孔面板里。

## Plasticity 里的结构

- `editor.assets` = `AssetPackManager`: `getAssetPacks()` → `[{filePath, path, lastOpened}]`(只是登记信息);
  `load(filePath)`(参数是**路径字符串**; 传对象报 path must be string)→ 读 pack 文件:
  `{schemaVersion, id, revision, exportedAt, unit, bufferViews[], assets[]}`, 每个资产
  `{id: 'node:…', type, subtype: 'group', name, path: [文件夹], thumbnail(dataURL), bbox(米), pivot: {position, orientation}, contentHash}`。只读, 不进撤销栈。
- 用户的螺丝 pack: 12 个(M1.4 … M20, 文件夹「杯头内六角」), 每个资产 = 螺钉(如 `杯内六M4x10`)+ **负体**(`杯内六M4Neg`, 减出孔:
  M4 = ⌀4.3 过孔段 + ⌀4 段 + ⌀3.3 底孔段)。

## 手工拖进视口发生了什么(透明记录器实测)

1. Assets 面板 `dragstart` → 画布 `drop`: `DataTransfer` 类型 **`plasticity/asset`** = JSON
   `{id, pivot, packId, packRevision, packAbsolutePath, contentHash}`。
2. 起 `ImportWithPlacementCommand`(字段 `assetFile / id / pivot / assetReference / origin`): 导入资产内容(不走几何工厂),
   `nodes.setAssetReference(节点, {packId, packRevision, packPath(相对), packAbsolutePath, assetId, contentHash})`。
3. 等放置: 命令的 `promises`(4 个 `CancellablePromise`: `finish / cancel / interrupt`)。
4. `PlaceFactory` commit: `origin`(= pivot)/ `originOrientation` → `destination`(落点)/ `destinationOrientation`(资产 +Z 对着落点面的外法向)。
   整个拖放 = 一个撤销步。

## 自动放置(垫片 `asset.place`)

合成同样的 drop(`new DataTransfer()` + `setData('plasticity/asset', …)` + `DragEvent` dragenter/dragover/drop 打在画布中心)→
等**新的** `ImportWithPlacementCommand`(看 `executor.active.command`; `lastCommand` 在上一条被取消后还指着旧的)→
合成一次 `pointermove`(拾取器没有当前点时 `finish()` 什么也不做)→ 临时给 `PlaceFactory.prototype` 挂一个 commit, 把
destination / destinationOrientation 换成要的(`setFromUnitVectors(+Z, normal)` + 绕 normal 转 rotate°)→ 对命令的 promises 逐个 `finish()` →
撤销栈 +1 = 放好 → 删掉临时 commit。可选 `cut`: 自建命令里 `BooleanFactory`(setter `targets` / `tools`, `operationType` 默认 subtract)把负体从目标减掉。
失败: 取消命令(导入的东西随之消失)+ 撤回到放之前。

坑:
- **命令对象是 thenable**(有 `then`): 拿它去 resolve 自己的 Promise, 链会等命令结束 —— 命令在等我们放置 = 死锁。包一层 `{cmd}`。
- 窗口不可见时拒绝(拖放 / 拾取器要可见窗口)。
- pla-ui 与 pla-app / pla-mcp 的垫片版本要一致(pla-ui 会把页面里的垫片换回它自己的版本, 正在跑的调用会被打断)。

实测: M4 放到 (300, 50, 20) + 减进 40×40×20 的方块 → 0.98 s, 2 个撤销步; 方块上出现 ⌀4.3 / ⌀4 / ⌀3.3 三段孔, 螺钉头在面上方。
`hole.make {dry: true}` 只返回孔位(面中点 + offset / 草图点), 给"用负体开孔"用。

## 用在哪

- 打孔面板(「操作」→ 打孔)新增「放螺丝(asset pack)」: 选 pack 里的资产(和孔规格同名的一键选), 方式 = 打孔后放螺丝 / 用 pack 负体开孔
  (孔位同打孔: 面中点或选中的草图点; 螺丝 +Z = 面外法向)。
- pla-mcp: `model_assets`(列表)、`model_asset_place`(放 + 可选减负体)。

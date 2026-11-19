// pla-shim.js — PLAhelper 页面内垫片 window.__PLA__（F1，设计见 docs/know-how/cdp-shim-design.md）。
//
// 整个文件是一个函数表达式，由 Rust（pla_bridge::shim::install_expr）以 `<本文件>(<配置 JSON>)` 注入。
// 四条硬规则：
//   R1 Plasticity 内部结构只出现在 LOC / NAMES 两张表里（带备选路径），功能代码一律 get('键')。
//   R2 能力 = 表键的组合（CAPS），探测自动生成；失效能力调用返回 cap:<名>，绝不静默。
//   R3 对外只有 def() 登记的接口名；Rust 侧 shim::api 常量与之一一对应（契约测试保证）。
//   R4 全局名、编辑器特征、选择器、命令名等常量由配置 CFG 传入，这里不另写一份。
(function (CFG) {
  'use strict';

  // 接口契约版本：破坏性改动才 +1；必须与 Rust shim::API_VERSION 相等（契约测试强制）。
  var API_VERSION = 1;
  var W = window;

  var prev = W.__PLA__;
  if (prev && prev.rev === CFG.rev && prev.api === API_VERSION) return JSON.stringify({ installed: false, same: true });
  if (prev && typeof prev.dispose === 'function') { try { prev.dispose(); } catch (e) {} }

  // =====================================================================
  // 1. 内部结构表（唯一来源）。Plasticity 升级改名时，在对应行追加备选路径即可。
  //    键 -> [根, 备选路径...]
  //    根：ed = Editor；sel = 当前 Selection；vp = 视口（见 viewport()）；oc = 视口的轨道控制器；
  //        reg = 命令注册表(CommandRegistry)；click = 选择点击器；cs = changeSelection；
  //        exec = 命令执行器；km = KeymapManager；kb = 键盘；fac = 代理挂上的几何工厂类表(N2)。
  //    方法类表键一律挂在其所属对象的根下, 调用时 this = 该对象(见 invokeOn)。
  // =====================================================================
  var LOC = {
    'ed.selection':            ['ed', 'selection'],
    'ed.viewports':            ['ed', 'viewports'],
    'ed.activeViewport':       ['ed', '_activeViewport', 'activeViewport'],
    'ed.changeSelection':      ['ed', 'changeSelection'],
    'cs.onClick':              ['cs', 'onClick'],
    'ed.geo':                  ['ed', 'geo'],
    'geo.lookupTopo':          ['geo', 'lookupTopologyItemById'],
    'geo.lookupItem':          ['geo', 'lookupItemById'],
    'ed.enqueue':              ['ed', 'enqueue'],
    // 物体名(大纲里显示的名字): nodes.getName(nodes.item2key(视图))。
    'ed.nodes':                ['ed', 'nodes'],
    'nodes.getName':           ['nodes', 'getName'],
    'nodes.item2key':          ['nodes', 'item2key'],
    // 选择命令基类 SelectionCommand 的来源: 任一已注册的选择类命令(原生点击命令类只在闭包里)。
    'cmd.selectionBaseHolder': ['ed', 'commands.DeselectAllCommand'],
    'sig.commandEnded':        ['ed', 'selection.signals.commandEnded', 'signals.commandEnded'],
    'ed.executor':             ['ed', 'executor'],
    'exec.cancel':             ['exec', 'cancelActiveCommand'],
    'ed.undoStack':            ['ed', 'history._undoStack', 'history.undoStack'],
    'ed.geoVersion':           ['ed', 'geo._version', 'geo.version'],
    'ed.errorSignal':          ['ed', 'signals.environmentChanged'],
    'ed.registry':             ['ed', 'registry'],
    'reg.add':                 ['reg', 'add'],
    'reg.listeners':           ['reg', 'inlineListenersByCommandName'],
    'ed.keymaps':              ['ed', 'keymaps'],
    'km.add':                  ['km', 'add'],
    'km.bindings':             ['km', 'keyBindings'],
    'ed.keyboard':             ['ed', 'keyboard'],
    'kb.update':               ['kb', 'updateKeymap'],
    'ed.settings.orbit':       ['ed', 'settings.OrbitControls'],
    'orbitSettings.step':      ['orbitSettings', 'rotateDiscreteIncrement'],
    'ed.shading.mode':         ['ed', 'shading.mode'],
    'ed.shading.section':      ['ed', 'shading._sectionMode'],
    'ed.shading.changed':      ['ed', 'shading.signals.shadingChanged'],
    'ed.sections':             ['ed', 'sections.sections'],
    // 官方 Bridge(8980)服务端: ws 库 WebSocketServer, _state 0=运行 1=关闭中 2=已关闭。
    'ed.bridge.state':         ['ed', 'websocket.server._state'],
    'ed.bridge.port':          ['ed', 'websocket.server.options.port', 'settings.Server.port'],
    'ed.bridge.enabled':       ['ed', 'settings.Server.enabled'],
    'sel.selected':            ['ed', 'selection.selected'],
    'sel.size':                ['sel', 'size'],
    // D 导出: 往选择里直接加 / 清空(导出范围 = 当前选择), 导出入口(无参, 弹"另存为")
    'sel.add':                 ['sel', 'add'],
    'sel.removeAll':           ['sel', 'removeAll'],
    'ed.export':               ['ed', 'export'],
    'vp.camera':               ['vp', 'camera'],
    'vp.orbit':                ['vp', 'orbitControls'],
    'vp.container':            ['vp', 'container'],
    'vp.render':               ['vp', 'setNeedsRender'],
    'vp.navigate':             ['vp', 'navigateToSelection'],
    'vp.cplane':               ['vp', '_constructionPlane'],
    'vp.settings.renderer':    ['vp', 'settings.Viewport.renderer'],
    'vp.settings.ao':          ['vp', 'settings.Viewport.ao'],
    'vp.settings.grid':        ['vp', 'settings.Grid'],
    'oc.quaternion':           ['oc', 'quaternion'],
    'oc.quaternionEnd':        ['oc', 'quaternionEnd'],
    'oc.step':                 ['oc', 'rotateDiscreteIncrement'],
    'oc.update':               ['oc', 'update'],
    'oc.domElement':           ['oc', 'domElement'],
    'oc.needsUpdate':          ['oc', 'needsUpdate'],
    'oc.setNeedsUpdate':       ['oc', 'setNeedsUpdate'],
    // ---- R33: 视口状态 v2(know-how/viewport-state.md) ----
    'vp.snapshot':             ['vp', 'snapshot'],
    'vp.init':                 ['vp', 'init'],
    'vp.isRenderMode':         ['vp', 'isRenderMode'],
    'vp.toggleRenderMode':     ['vp', 'toggleRenderMode'],
    'vp.togglePerspective':    ['vp', 'togglePerspective'],
    // 缩略图: 同一任务里"渲一帧 → 读 WebGL canvas"(不走 CDP Page.captureScreenshot —— 它会让整个窗口闪黑)
    'vp.handleRender':         ['vp', 'handleRender'],
    'vp.needsRender':          ['vp', 'needsRender'],
    'vp.canvas':               ['vp', 'renderer.domElement'],
    'ed.layers':               ['ed', 'layers'],
    'ed.docSettings':          ['ed', 'document.settings'],
    'ed.originator':           ['ed', 'originator'],
    'ed.shadingDb':            ['ed', 'shading'],
    'ed.sig.docSettings':      ['ed', 'signals.documentSettingsChanged'],
    'nodes.state':             ['nodes', 'read'],
    'nodes.isolate':           ['nodes', 'isolate'],
    'nodes.unisolate':         ['nodes', 'unisolate'],
    'orig.snapshot':           ['orig', 'snapshot'],
    'orig.restore':            ['orig', 'restore'],
    // R34d: 材质库的纹理加载器(按文件名缓存 Promise<Texture>); 恢复带贴图的材质时用它重建真 Texture
    'mat.textures':            ['ed', 'materials.textures'],
    // 材质库本体(根 mats): snapshot() = { version, counter, materials }, 与 memento 的 materials 段同形; 有命令进行中也能读
    'ed.materialsDb':          ['ed', 'materials'],
    // 组(大纲里的文件夹 = 部件 / 子装配): getParent(nodeKey) → 组对象; 组名 = nodes.getName(item2key(组))
    'ed.groupsDb':             ['ed', 'groups'],
    'groups.getParent':        ['groupsDb', 'getParent'],
    'mats.snapshot':           ['mats', 'snapshot'],
    'tex.get':                 ['tex', 'get'],
    'ds.snapshot':             ['ds', 'snapshot'],
    'ds.init':                 ['ds', 'init'],
    'sh.snapshot':             ['sh', 'snapshot'],
    'sh.init':                 ['sh', 'init'],
    'sh.setMatcap':            ['sh', 'setMatcap'],
    // 视口截面(shading.section = Section; isSectioned 的 setter 同时切换截面模式)
    'sh.isSectioned':          ['sh', 'isSectioned'],
    'sh.section':              ['sh', 'section'],
    // 渲染环境的旋转 / 强度(setter); 环境本身只能点渲染面板的缩略图按钮切换
    'sh.envRotation':          ['sh', 'environmentRotation'],
    'sh.envIntensity':         ['sh', 'environmentIntensity'],
    // ---- N2: 命令派发(know-how/command-dispatch.md) ----
    'geo.items':               ['geo', 'items'],
    'ed.docUnit':              ['ed', 'document.settings._Unit.length'],
    // 自建工厂命令的基类 Command: 取任一几何命令类的父类
    'cmd.factoryBaseHolder':   ['ed', 'commands.CornerBoxCommand'],
    // 几何工厂类(只在闭包里; 代理按需拍堆快照挂到 CFG.factoriesGlobal, 根 fac)
    // ---- N3: 面高亮(叠加在辅助场景里的自建网格; 不动选择、不受鼠标悬停覆盖) ----
    'ed.helpers':              ['ed', 'helpers'],
    'helpers.add':             ['helpers', 'add'],
    'helpers.remove':          ['helpers', 'remove'],
    'gizmo.meshMaterial':      ['ed', 'gizmos.yellow.mesh', 'gizmos.default.mesh'],
    // ---- A2: 吸附点(测量 / 自由移动命令用的同一批点; SnapScene 按物体缓存, 新物体建完即有) ----
    'ed.snapScene':            ['ed', 'snapScene'],
    'snap.canonicals':         ['ss', 'canonicals'],
    'snap.occurrences':        ['ss', 'occurrences'],
    'fac.cornerBox':           ['fac', 'CornerBoxFactory'],
    'fac.multiFillet':         ['fac', 'MultiFilletShellFactory'],
    'fac.offsetFace':          ['fac', 'OffsetFaceFactory'],
    'fac.refillet':            ['fac', 'RefilletFaceFactory'],
    'fac.moveFaces':           ['fac', 'MultiMoveFaceFactory'],
    'fac.cylinder':            ['fac', 'CylinderFactory'],
    // ---- A4: 草图曲线 → 区域 → 拉伸 / 旋转(实测 2026-11-17, know-how/command-dispatch.md「A4」) ----
    'fac.centerPolygon':       ['fac', 'CenterPolygonFactory'],
    'fac.curve':               ['fac', 'CurveFactory'],
    'fac.extrude':             ['fac', 'ExtrudeFactory'],
    'fac.revolve':             ['fac', 'RevolveFactory'],
    // ---- A4b: 整体操作(复制 / 镜像 / 移动 / 旋转 / 阵列, 实测 2026-11-19, know-how/command-dispatch.md「A4b」) ----
    'fac.duplicate':           ['fac', 'DuplicateFactory'],
    'fac.mirror':              ['fac', 'MirrorFactory'],
    'fac.moveItems':           ['fac', 'MoveItemAndEmptyFactory'],
    'fac.rotateItems':         ['fac', 'RotateItemAndEmptyFactory'],
    'fac.rectArray':           ['fac', 'RectangularArrayFactory'],
    'fac.radialArray':         ['fac', 'RadialArrayFactory'],
    'fac.boolean':             ['fac', 'BooleanFactory'],
    'geo.removeItem':          ['geo', 'removeItem'],
    // ---- N5 资产(asset pack)放置(实测 2026-11-18, know-how/asset-place.md) ----
    'ed.assets':               ['ed', 'assets'],
    'assets.packs':            ['assets', 'getAssetPacks'],
    'assets.add':              ['assets', 'add'],
    'assets.remove':           ['assets', 'remove'],
    'nodes.setName':           ['nodes', 'setName'],
    'nodes.setPivot':          ['nodes', 'setPivot'],
    'nodes.isAsset':           ['nodes', 'isAsset'],
    'nodes.findAllAssets':     ['nodes', 'findAllAssets'],
    'cmd.toggleAsset':         ['ed', 'commands.ToggleAssetCommand'],
    'cmd.exportAssetPack':     ['ed', 'commands.ExportAssetPackCommand'],
    'assets.load':             ['assets', 'load'],
    'exec.last':               ['exec', 'lastCommand'],
    // 正在执行的命令(lastCommand 在上一条被取消后可能还指着旧的)
    'exec.activeCmd':          ['exec', 'active.command'],
    'fac.place':               ['fac', 'PlaceFactory'],
    // ---- N3e: 本体测量(Measure Distance 的同一工厂; 名字在 nodes 表里, 不在测量记录上 —— 实测 2026-11-15) ----
    'ed.measurements':         ['ed', 'measurements'],
    'meas.snapshot':           ['meas', 'snapshot'],
    'nodes.setName':           ['nodes', 'setName'],
    'fac.p2pMeasure':          ['fac', 'PointToPointMeasurementFactory'],
    // ---- 大纲(文件夹 = 组; 实测 2026-11-19 后台副本): 节点键 = id × 256 + 类别(0 组 / 1 物体);
    //      create() → 新组(尚未挂在树上) → addMembership(组键, 父组) 挂上; 移动 = _unsafeMoveNodeToGroup(键, 目标组)
    //      (addMembership 对已在别处的节点只加不删, 旧父组里留下残影 → 不用它移动); dissolve(组) = 解散(成员回父组)。 ----
    'groups.create':           ['groupsDb', 'create'],
    'groups.attach':           ['groupsDb', 'addMembership'],
    'groups.move':             ['groupsDb', '_unsafeMoveNodeToGroup'],
    'groups.dissolve':         ['groupsDb', 'dissolve'],
    'groups.children':         ['groupsDb', 'getChildren'],
    'groups.byId':             ['groupsDb', 'lookupById'],
    'groups.parentId':         ['groupsDb', 'getParentId'],
    'groups.root':             ['groupsDb', 'root'],
    'groups.all':              ['groupsDb', 'all'],
    'nodes.key2item':          ['nodes', 'key2item'],
    'nodes.setHidden':         ['nodes', 'setHidden'],
    'nodes.isHidden':          ['nodes', 'isHidden'],
    'nodes.isVisible':         ['nodes', 'isVisible'],
    'nodes.isLocked':          ['nodes', 'isLocked'],
    // 改完隐藏 / 组成员后要重算场景可见性(SceneVisibilityManager.update; 实测: 不调则视口照旧显示, 命令结束也不会自动重算)
    'ed.scene':                ['ed', 'scene'],
    'scene.update':            ['scene', 'update']
  };

  // 选择集里的各类 id 集合：[对外的 kind, Selection 上的集合名]。sel.ids.<kind> 表键由此生成。
  var SEL_KINDS = [
    ['solid', 'solidIds'], ['sheet', 'sheetIds'], ['face', 'faceIds'], ['edge', 'edgeIds'],
    ['segment', 'segmentIds'], ['curve', 'curveIds'], ['curveVertex', 'curveVertexIds'],
    ['group', 'groupIds'], ['plane', 'planeIds']
  ];
  SEL_KINDS.forEach(function (k) { LOC['sel.ids.' + k[0]] = ['sel', k[1]]; });

  // 轨道控制器的阻尼量：[对外字段名, 当前值属性, 目标值属性, 类型]。oc.<名> / oc.<名>End 表键由此生成。
  // 阻尼：每帧把 x 往 xEnd 插值，写入时两份都要写（pitfalls / R10）。
  var ORBIT_FIELDS = [
    ['target', 'target', 'targetEnd', 'v3'],
    ['focal', 'focalOffset', 'focalOffsetEnd', 'v3'],
    ['radius', 'radius', 'radiusEnd', 'num'],
    ['zoom', 'zoom', 'zoomEnd', 'num']
  ];
  ORBIT_FIELDS.forEach(function (f) {
    LOC['oc.' + f[0]] = ['oc', f[1]];
    LOC['oc.' + f[0] + 'End'] = ['oc', f[2]];
  });

  // 非对象路径的内部名字（DOM / Preact 组件名）。
  var NAMES = {
    appRootId: 'root',
    vnodeChildren: '__k',
    outlinerComponent: 'Outliner',
    outlinerSections: ['OutlinerSection', 'Memoed'],
    vnodeScanLimit: 80000,
    // 命令进行中 Plasticity 挂在 <body> 上的属性(取消命令后要清掉, 否则悬停管线不恢复)。
    bodyBusyAttrs: ['gizmo', 'command'],
    // 拓扑项的版本 id 字段。
    versionIdKey: 'versionId',
    // sel.apply: 支持的成员类型; 其中按拓扑项查(lookupTopologyItemById)的类型, 其余按物体查(lookupItemById)。
    applyKinds: ['face', 'edge', 'solid', 'sheet', 'curve'],
    topologyKinds: ['face', 'edge'],
    // 几何库查找结果里视图对象 / 几何模型的字段。
    lookupViewKey: 'view',
    lookupModelKey: 'model',
    // 拓扑项视图上指向所属物体(实体/片体)的字段。
    parentItemKey: 'parentItem',
    // 面模型取曲面: model.GetSurface() → { surface: <Plane|Cylinder|Sphere|Torus|BlendSurf…>, … }。
    surfaceGetter: 'GetSurface',
    surfaceInnerKey: 'surface',
    // 选择项描述缓存上限(按版本 id 缓存: 几何一变版本 id 就变, 不会过期)。
    describeCacheLimit: 5000,
    // ChangeSelectionModifier(实测): 0 替换 / 1 追加 / 2 移除 / 3 切换。
    modReplace: 0,
    modAdd: 1,
    // three.js 相机字段。
    perspectiveFlag: 'isPerspectiveCamera',
    // 着色模式的标签字段与切换时需要的 prev 结构字段。
    shadingTagKey: 'tag',
    shadingPrevField: 'prev',
    shadingPrevKeys: ['materials', 'isTinted'],
    // Atom KeymapManager 的 KeyBinding 字段。
    keyBindingFields: { selector: 'selector', keys: 'keystrokes', command: 'command' },
    // 常驻维护(补注册滚转命令等)的最小间隔(ms)。
    maintainEveryMs: 2000,
    // 剖面 / 构造面的向量字段。
    planeVecKeys: ['p', 'n', 'x', 'y'],
    rendererKeys: ['antiAlias', 'toneMapping', 'toneMappingExposure'],
    gridKeys: ['every', 'size'],
    cameraExtraKeys: ['fov', 'zoom', 'near', 'far'],
    // ---- R33: 视口状态 v2 ----
    vpStateVersion: 2,
    // 视口上可读写的显示标量(getter/setter)。不含 fov: 视口 fov 不是视场角而是投影开关(透视 1 / 正交 0),
    // 写它(任何值)都会切成透视 —— 投影只由 view 段(togglePerspective)负责(用户实测 2026-11-15, pitfalls #58)。
    vpScalars: ['squish', 'curvatureScale', 'g0Scale', 'g1Scale', 'g2Scale'],
    // LayerManager 的显示开关(getter/setter)。
    layerFlags: ['isShowingEdges', 'isShowingFaces', 'isShowingCurves', 'isShowingMeshes', 'isShowingImages',
                 'isShowingConstraints', 'isShowingControlPoints', 'isShowingContinuityLabels'],
    // NodeState(nodes.read)里的可见性集合 + nodeKey → 稳定 id 的映射。
    nodeVisSets: ['hidden', 'invisible'],
    nodeIsolations: 'isolations',
    node2id: 'node2id',
    // memento 里 nodes 段的字段名(与 nodeVisSets 同名, 值是 Float64Array)。
    mementoNodes: 'nodes',
    // matcap 纹理上记来源文件的字段(texture.userData.filename)。
    matcapFileKey: 'filename',
    // Shader Mode 面板: 标题文字 + 按钮激活态属性。着色恢复必须点按钮(无按名查找的注册表; 在 shader
    // 模式下直接 setMatcap 会让 matcap 变 undefined → 渲染循环停摆, 见 know-how/viewport-state.md)。
    shaderPanelTitle: 'SHADER MODE',
    shaderButtonStateAttr: 'data-state',
    shaderButtonActive: 'active',
    // shader 名 → 面板按钮图片名; matcap 按钮图片名 = matcap 文件名去扩展名。
    shaderButtons: { isophotes: 'isophotes', draft: 'draft', zebraHorizontal: 'reflection_check_horizontal',
                     zebraVertical: 'reflection_check_vertical', topo: 'topo', faceDirection: 'face-direction', curvature: 'curvature' },
    // 这几个按钮共用 basic_side matcap, 内部状态分不开 → 存档时以面板激活态为准。
    sharedMatcapButtons: ['color_matcap', 'black_silhouette', 'color_silhouette'],
    sharedMatcapFile: 'basic_side',
    // 渲染环境按钮图片名 = 环境文件名去扩展名再去分辨率后缀(outdoor_cloudy_01.png ↔ outdoor_cloudy_01_1k.exr)。
    envResSuffix: /_\d+k$/,
    // Section 对象的参数: 基准平面 / 基准位置 + transform(distance, angle1, angle2, flip)。
    sectionBasePlane: 'originalPlane',
    sectionBasePos: 'originalPosition',
    sectionTransform: 'transform',
    sectionParams: ['distance', 'angle1', 'angle2', 'flip'],
    // ---- R34: 材质状态(memento 字段) ----
    mementoMaterials: 'materials',      // memento.materials = { version, counter, materials: [...] }
    materialsList: 'materials',
    materialsCounter: 'counter',
    materialId: 'id',
    materialUuid: 'uuid',
    mementoNodeMaterials: 'materials',  // memento.nodes.materials = [nodeKeys(类型化数组), materialIds]
    node2material: 'node2material',     // nodes.read.node2material: nodeKey → materialId
    // 材质的贴图槽: { repeat, texture: THREE.Texture }(normalMap / roughnessMap …); 文件名在 texture.userData(同 matcapFileKey)
    materialTexSlot: 'texture',
    // ---- N2: 命令派发 ----
    // 工厂实例方法: resource(命令) 随命令释放; commit() 生成几何(返回新物体视图数组)
    facResource: 'resource',
    facCommit: 'commit',
    solidCtor: 'Solid',
    // 实体视图上的面组/边组; 组里 versionIds(拓扑 id '<物体>f<n>' / '<物体>e<n>')与逐项 bboxes(6 个数一项)同序
    shellGroups: { face: 'faces', edge: 'edges' },
    groupVersionIds: 'versionIds',
    groupBboxes: 'bboxes',
    // 物体视图的版本 id(= lookupItemById 的 id): view.userData.versionId
    itemUserData: 'userData',
    // 文档长度单位 → 内部单位(米)的倍数
    unitScale: { millimeter: 0.001, centimeter: 0.01, meter: 1, inch: 0.0254, foot: 0.3048 },
    // 布尔运算(Parasolid PK_boolean_*_c; union 实测, 另两个按同一枚举)
    boolOps: { union: 15903, subtract: 15902, intersect: 15901 },
    boolSetters: { op: 'operationType', targets: 'targets' },
    // ---- A1: 几何查询(Parasolid 封装对象上的只读方法与结果字段, 26.2.3 实测) ----
    geoApi: {
      surfaceKey: 'surface',            // face.GetSurface() → { surface, orientation, uMin… }
      midpoint: 'FindMidpoint',         // face.FindMidpoint() → { position, normal }(法向朝外)
      midPos: 'position', midNormal: 'normal',
      info: 'GetInfo',                  // 曲面/曲线 GetInfo(): Cylinder/Circle → { radius, basis }; Line → { origin, direction }
      infoRadius: 'radius', infoBasis: 'basis', lineOrigin: 'origin', lineDir: 'direction',
      basisLoc: 'Location', basisAxis: 'Axis',
      curve: 'GetCurve', curveKey: 'curve',   // edge.GetCurve() → { curve: Line|Circle|…, sense, uMin, uMax }
      length: 'FindLength', lengthKey: 'length', // edge.FindLength() → { length, min, max }
      vertices: 'GetVertices', vLeft: 'left', vRight: 'right', // edge.GetVertices() → { left, right }
      point: 'GetPoint', ident: 'GetIdentifier',
      blendRadius: 'GetBlendRadius'     // 倒圆面 → 半径(米); 普通圆柱 → −1(实测)
    },
    // 顶点没有 Plasticity 拓扑 id(lookupTopologyItemById 不认) → 自拟 '<物体版本id>v<顶点实体id>'(实体 id 取自吸附数据,
    // 与面/边拓扑 id 里的数字同一套; 没有吸附数据时退回 Parasolid 标识)
    vertexIdTag: 'v',
    // ---- A2: 吸附数据(SnapScene.canonicals: 'Item:<物体版本id>|<bool>' → { snapsForItem, positions, box }) ----
    snapKeyPrefix: 'Item:',
    snapItemKey: 'snapsForItem',
    // 每类吸附点: 对外 kind → [snapsForItem 下的路径, 拓扑 id 字段, 拓扑类型, 位置字段, 方向字段, 方向的对外名]
    snapKinds: {
      faceCenter:    ['faces.centers', 'faceIds', 'face', 'positions', 'normals', 'normal'],
      edgeMid:       ['edges.halves', 'edgeIds', 'edge', 'positions', 'tangents', 'tangent'],
      circleCenter:  ['edges.circles', 'edgeIds', 'edge', 'centers', 'axes', 'axis'],
      circleQuarter: ['edges.circles', 'edgeIds', 'edge', 'quarters', 'tangents', 'tangent'],
      vertex:        ['shellVertices', 'vertexIds', 'vertex', 'positions', null, null],
      bboxCorner:    ['bbox.corners', null, null, 'positions', 'normals', 'normal']
    },
    topoTag: { face: 'f', edge: 'e', vertex: 'v' },
    // 实例变换(occurrence.transform: 16 个数, 列主序 = three.js Matrix4.elements)
    occEntry: 'entry', occTransform: 'transform',
    // 面积 / 体积 / 质心: 内核没有对外方法(26.2.3 只有 GetCentroid = 包围盒中心) → 用显示网格算:
    // FaceGroup.mesh.geometry(position + index), groups = [起点, 数量] × 面(与 versionIds 同序)。曲面是近似(R3 圆角面 −0.3%)。
    faceMesh: 'mesh', meshGroups: 'groups',
    // ---- A5: 视角截图 ----
    // 视角 → 相机朝向(目标 → 相机)+ 上方向(世界 Z 朝上; 顶/底视图以 +Y 为上)。iso = 右前上(同 SW 等轴)
    camViews: {
      front:  { dir: [0, -1, 0], up: [0, 0, 1] }, back:   { dir: [0, 1, 0], up: [0, 0, 1] },
      right:  { dir: [1, 0, 0], up: [0, 0, 1] },  left:   { dir: [-1, 0, 0], up: [0, 0, 1] },
      top:    { dir: [0, 0, 1], up: [0, 1, 0] },  bottom: { dir: [0, 0, -1], up: [0, 1, 0] },
      iso:    { dir: [1, -1, 1], up: [0, 0, 1] }
    },
    orthoMode: 'orthographic',
    // 正交相机: 可见半高 = top / zoom(ProxyCamera 的 top 固定 3); aspect = 宽 / 高
    camOrthoTop: 'top', camAspect: 'aspect',
    viewMargin: 1.15,
    // 判定两平面"平行"的角度容差(度): 平行时 measure 额外给法向距离(壁厚); 平面的曲面类名
    parallelTolDeg: 0.1,
    planeSurface: 'Plane', cylinderSurface: 'Cylinder', coneSurface: 'Cone',
    // ---- N3: 面高亮的样式(角色 → 颜色 / 不透明度); 透过实体可见(depthTest 关), 画在最上层 ----
    // 颜色来自配置(Rust shim::HIGHLIGHT_*; 面板色块用同一份), 这里只定不透明度
    highlightStyles: {
      primary:   { color: (CFG.highlightColors || {}).primary, opacity: 0.55 },     // 当前对面 / 锚面
      candidate: { color: (CFG.highlightColors || {}).candidate, opacity: 0.25 },   // 其它候选
      source:    { color: (CFG.highlightColors || {}).source, opacity: 0.35 }       // 参考面
    },
    highlightRenderOrder: 999,
    // geo.query 每类最多返回的项数(超出如实报 truncated)
    queryLimit: 2000,
    // ---- A3: 事务回滚 ----
    // 撤销 = 在 document.body 上派发 edit:undo(与 Ctrl+Z 同一条路径; 异步, 实测 300 ms 内完成; 内部走 memento 恢复)
    undoCommand: 'edit:undo',
    // 退出隔离 = Plasticity 自己的命令(键位 alt-. → command:unisolate, 选择器 body); 恢复可见性时用它, 不直接写 NodeState 隔离层
    unisolateCommand: 'command:unisolate',
    undoPollMs: 50,
    undoStepTimeoutMs: 5000,
    // 每步命令的超时(ms); 按位置挑边/面的默认容差(文档单位)
    cmdTimeoutMs: 15000,
    // 命令入队后多久还没开始执行就重新入队、最多几次(runCommand 看门狗)
    cmdStartWatchMs: 1500,
    cmdStartRetries: 2,
    pickTolerance: 0.5,
    // 配方表(唯一来源): 对外操作名 → 工厂表键 + 参数(对外名 → [工厂 setter, 类型]) + 必填 + 是否支持布尔。
    // 类型决定换算: point(文档单位 → 米, 构造 Vector3) / length / angle(度 → 弧度) / number / bool /
    //               edges / faces(拓扑 id 或 {near, of?, tol?}) / solids(物体 id / '$引用' / {name})。
    // 参数按这里的顺序设置。
    recipes: {
      box: { fac: 'fac.cornerBox', bool: true, required: ['p1', 'p2', 'height'],
             params: { p1: ['p1', 'point'], p2: ['p2', 'point'], height: ['height', 'length'] } },
      fillet: { fac: 'fac.multiFillet', required: ['edges', 'distance'],
                params: { edges: ['edges', 'edges'], distance: ['distance', 'length'] } },
      // N3: 面沿法向偏移(正 = 沿外法向长出去, 实测见 know-how/command-dispatch.md)
      offsetFace: { fac: 'fac.offsetFace', required: ['faces', 'distance'],
                    params: { faces: ['faces', 'faces'], distance: ['distance', 'length'] } },
      // N3: 倒圆面改半径(保持与两侧相切; distance = 半径增量, 实测 R2 + 3 → R5)
      refillet: { fac: 'fac.refillet', required: ['faces', 'distance'],
                  params: { faces: ['faces', 'faces'], distance: ['distance', 'length'] } },
      // N3c: 一组面整体平移(孔位 / 凸台位置 / 两面间距); move = [dx, dy, dz](文档单位)。
      // 实测要同时设 move(Vector3)与 matrix(平移矩阵): 只设 move 无效果, 只设 matrix 也无效果。
      moveFaces: { fac: 'fac.moveFaces', required: ['faces', 'move'],
                   params: { faces: ['faces', 'faces'], move: [null, 'translation'] } },
      // N4: 圆柱(+ 布尔; 打孔 = subtract)。center = 底面圆心, axis = 长出去的方向, height 沿 axis(实测 2026-11-15:
      // 设 center / orientation / 半径点 / radius / height; 孔 = axis 取入口面法向的反向)
      cylinder: { fac: 'fac.cylinder', bool: true, radiusPoint: true, required: ['center', 'axis', 'radius', 'height'],
                  params: { center: ['center', 'vecset'], axis: ['orientation', 'axisQuat'], radius: ['radius', 'length'], height: ['height', 'length'] } },
      // N4: 倒角(= 圆角工厂距离取负; 孔口倒角 = 锥形沉头)
      chamfer: { fac: 'fac.multiFillet', required: ['edges', 'distance'],
                 params: { edges: ['edges', 'edges'], distance: ['distance', 'negLength'] } },
      // A4: 正多边形柱(内六角孔 / 六角头 / 螺母): 中心 + 法向(拉伸方向, 默认 +Z)+ 边数 + 外接圆半径 radius 或对边距 flats + 高度(负 = 反向);
      // rotate = 绕法向转多少度(默认一个顶点朝 u 轴)。一步 = 画多边形 → 拉伸(+ 布尔), 一个撤销步; 辅助曲线保留。
      prism: { custom: 'prism', fac: 'fac.extrude', also: ['fac.centerPolygon'], bool: true, required: ['center', 'sides', 'height'],
               params: { center: [null, 'point'], normal: [null, 'axisQuat'], sides: [null, 'number'], radius: [null, 'length'], flats: [null, 'length'],
                         height: [null, 'length'], rotate: [null, 'angle'] } },
      // A4: 旋转体(螺钉 / 轴 / 台阶 / 锥): profile = [[r, z], …] —— r = 离轴距离(≥ 0), z = 沿 axis 的位置, 相对 origin(文档单位);
      // 自动闭合; 贴轴的边就是轴线上的线段。degrees 默认 360。
      revolve: { custom: 'revolve', fac: 'fac.revolve', also: ['fac.curve'], bool: true, required: ['origin', 'axis', 'profile'],
                 params: { origin: [null, 'point'], axis: [null, 'axisQuat'], profile: [null, 'profile'], degrees: [null, 'number'] } },
      // A4b 整体操作(实体; 产物 = 工厂 commit 的返回, 可用 id 引用)。duplicate = 原地复制一份(接 move / rotate 摆位置);
      // mirror = 按平面(过 origin、法向 normal)镜像出一份, 原件保留(原件版本 id 会变);
      // move = 平移 [dx,dy,dz]; rotate = 绕过 pivot、方向 axis 的轴转 degrees(右手定则);
      // pattern = 线性阵列: dir1 方向 count1 个(含原件)间距 spacing1, 可选第二方向 dir2 / count2 / spacing2(产物 = 新增的副本);
      // radialPattern = 绕过 center、方向 axis 的轴环形阵列 count 个(含原件), 均布在 degrees(默认 360)内:
      // 360 = 整圈等分(不重叠); 不足一圈时首尾都放(实测 model1 2026-11-19: 3 个 / 180° → 0° / 90° / 180°)。
      duplicate: { custom: 'duplicate', fac: 'fac.duplicate', required: ['solids'], params: { solids: [null, 'solids'] } },
      mirror: { custom: 'mirror', fac: 'fac.mirror', required: ['solids', 'origin', 'normal'],
                params: { solids: [null, 'solids'], origin: [null, 'vecset'], normal: [null, 'axisQuat'] } },
      move: { custom: 'move', fac: 'fac.moveItems', required: ['solids', 'move'],
              params: { solids: [null, 'solids'], move: [null, 'translation'] } },
      rotate: { custom: 'rotate', fac: 'fac.rotateItems', required: ['solids', 'pivot', 'axis', 'degrees'],
                params: { solids: [null, 'solids'], pivot: [null, 'vecset'], axis: [null, 'axisQuat'], degrees: [null, 'angle'] } },
      pattern: { custom: 'pattern', fac: 'fac.rectArray', required: ['solids', 'dir1', 'count1', 'spacing1'],
                 params: { solids: [null, 'solids'], dir1: [null, 'axisQuat'], count1: [null, 'number'], spacing1: [null, 'length'],
                           dir2: [null, 'axisQuat'], count2: [null, 'number'], spacing2: [null, 'length'] } },
      radialPattern: { custom: 'radialPattern', fac: 'fac.radialArray', required: ['solids', 'center', 'axis', 'count'],
                       params: { solids: [null, 'solids'], center: [null, 'vecset'], axis: [null, 'axisQuat'], count: [null, 'number'], degrees: [null, 'number'] } },
      // boolean = 已有实体之间布尔: targets 被 tools 合并 / 减去 / 求交(operation; 默认 tools 被吃掉, keepTools 保留)。
      // 实测 2026-11-19: intersect 两个盒子 → 只剩重叠部分一个实体。
      boolean: { custom: 'boolean', fac: 'fac.boolean', required: ['targets', 'tools', 'operation'],
                 params: { targets: [null, 'solids'], tools: [null, 'solids'], operation: [null, 'boolOp'], keepTools: [null, 'bool'] } },
      // delete = 删除实体(同 Plasticity 的 Delete, 一个撤销步; 实测 model1: 撤销后实体回来)
      delete: { custom: 'delete', fac: 'geo.removeItem', required: ['solids'], params: { solids: [null, 'solids'] } },
      // rename = 改大纲名(多个实体同名; 复制件改规格后改名, BOM 名称跟着对)
      rename: { custom: 'rename', fac: 'nodes.setName', required: ['name'], params: { solids: [null, 'solids'], folders: [null, 'folders'], name: [null, 'string'] } },
      // 大纲(2026-11-19): folder = 新建文件夹(parent 默认顶层)并把 solids / folders 移进去; moveTo = 移到文件夹 to('root' = 顶层);
      // hide / show = 同 H / 取消隐藏(文件夹整个); ungroup = 解散文件夹(成员回上一级)。都在我们自己的命令里做 = 一个撤销步。
      // 文件夹引用: 组 id(数字) / 名字(字符串或 {name}, 重名报错) / '$步骤id' / 'root'。
      folder: { custom: 'folder', fac: 'groups.create', also: ['groups.attach', 'groups.move', 'nodes.setName'], required: ['name'],
                params: { name: [null, 'string'], parent: [null, 'folder'], solids: [null, 'solids'], folders: [null, 'folders'] } },
      moveTo: { custom: 'moveTo', fac: 'groups.move', required: ['to'], params: { to: [null, 'folder'], solids: [null, 'solids'], folders: [null, 'folders'] } },
      hide: { custom: 'hide', fac: 'nodes.setHidden', params: { solids: [null, 'solids'], folders: [null, 'folders'] }, required: [] },
      show: { custom: 'show', fac: 'nodes.setHidden', params: { solids: [null, 'solids'], folders: [null, 'folders'] }, required: [] },
      ungroup: { custom: 'ungroup', fac: 'groups.dissolve', required: ['folders'], params: { folders: [null, 'folders'] } }
    },
    // 大纲节点的类别(节点键 % 256)与物体类(key2item 的构造名)→ 对外 kind
    outline: { groupCtor: 'Groups_Group', rootRef: 'root', ops: ['folder', 'moveTo', 'hide', 'show', 'ungroup'],
               kinds: { Solid: 'solid', Sheet: 'sheet', Wire: 'curve', InstanceEmpty: 'instance', Plane: 'plane',
                        DistanceMeasurement: 'measurement', AngleMeasurement: 'measurement', Image: 'image', Mesh: 'mesh' } },
    // A4b 工厂字段(实测 2026-11-19): 复制 / 移动 / 旋转 / 阵列的物体列表 = items, 镜像 = shells; 镜像 origin / normal、移动 move、
    // 旋转 pivot(原地 set)+ axis(VectorProxy, 原地 copy)+ angle(弧度); 线性阵列 dir1 / dir2(新 Vector3)+ num1 / num2 +
    // distance1 / distance2(mode extent = 首尾总长); 环形阵列 dir1 = 起始方向(⟂ 轴)、dir2 = 轴、center、num2 = 个数、angle = 总角度。
    xformFields: {
      items: 'items', mirrorItems: 'shells', origin: 'origin', normal: 'normal', move: 'move',
      pivot: 'pivot', axis: 'axis', angle: 'angle',
      dir1: 'dir1', dir2: 'dir2', num1: 'num1', num2: 'num2', dist1: 'distance1', dist2: 'distance2',
      center: 'center', radialCount: 'num2', radialAngle: 'angle',
      // 布尔工厂: targets / operationType 同 boolSetters, 另有 tools / keepTools
      tools: 'tools', keepTools: 'keepTools'
    },
    // N4b 打孔位置: 选择里可当孔位的类型(曲线端点 / 整条曲线); Wire 视图上的端点组(ids + Points 几何)与线段组
    holePointKinds: { vertex: 'curveVertex', curve: 'curve' },
    wire: { vertices: 'vertices', vertexPositions: 'points.geometry.attributes.position.array', segments: 'segments' },
    // N4 圆柱工厂的实例字段: 圆心 / 半径点(Vector3)、半径 setter; 朝向四元数把 baseAxis 转到圆柱轴
    cylinderFields: { center: 'center', point: 'point', radius: 'radius', baseAxis: [0, 0, 1] },
    // translation 类型参数写到工厂的哪两个字段(Vector3.set + Matrix4.makeTranslation)
    translationFields: { vector: 'move', matrix: 'matrix' },
    // A4: 多边形 / 折线 / 拉伸 / 旋转工厂的字段(实测 2026-11-17)。折线 type 0(默认 1 是过点样条, 闭合时会鼓出去);
    // 点用 push(Vector3); 闭合曲线提交后同一命令里就有新的 SketchIsland(区域 id 形如 '94r2669675', lookupTopologyItemById → Region 视图)。
    // 拉伸: regions + direction(单位向量, 原地 set)+ distance1(米); 旋转: regions + origin / axis(原地 set)+ degrees setter(轴要在轮廓平面里、轮廓不能跨轴)。
    // N5 资产放置(实测 2026-11-18): Assets 面板拖进视口 = 画布上 drop 事件, DataTransfer 'plasticity/asset' = JSON
    // {id, pivot, packId, packRevision, packAbsolutePath, contentHash} → ImportWithPlacementCommand 导入并挂资产引用, 然后等放置
    // (promises: CancellablePromise, finish() = 按当前状态确定)→ PlaceFactory 从 pivot 挪到 destination / destinationOrientation(资产 +Z 对着面外法向)。
    // pack 内容: assets.load(pack.filePath) → {id, revision, assets: [{id, name, path(文件夹), pivot, bbox(米), contentHash, thumbnail}]}。
    // 孔径一律按 pla-ui 的国标目录(用户 2026-11-18: pack 里不再放负体)。
    assetDnd: {
      mime: 'plasticity/asset', importCmd: 'ImportWithPlacementCommand', promises: 'promises', finish: 'finish',
      dest: 'destination', destQ: 'destinationOrientation', baseNormal: [0, 0, 1],
      pack: { id: 'id', revision: 'revision', path: 'filePath', assets: 'assets' },
      asset: { id: 'id', name: 'name', folder: 'path', pivot: 'pivot', bbox: 'bbox', hash: 'contentHash' },
      startWaitMs: 3000, placeWaitMs: 6000, moveSettleMs: 150,
      // 建库 setPivot 要真的 Vector3 / Quaternion(给数组时导出静默失败, 实测 2026-11-18); 类从视图的这两个字段取
      pivotCtors: { vec: 'position', quat: 'quaternion' }
    },
    sketchFields: {
      polygon: { center: 'center', point: 'point', orientation: 'orientation', sides: 'vertexCount', radius: 'radius', baseNormal: [0, 0, 1] },
      curve: { type: 'type', polyline: 0, push: 'push', closed: 'closed' },
      sweep: { regions: 'regions', direction: 'direction', distance: 'distance1', origin: 'origin', axis: 'axis', degrees: 'degrees' },
      islandCtor: 'SketchIsland', islandRegions: 'regions'
    },
    // ---- B1/B2 BOM: 指纹比较的相对容差(网格近似, 旋转后的复制件有 Float32 噪声); 手性阈值(相对 面积 × 尺度³);
    //      组: 根组 id 与组对象的 id 字段(ed.groups.getParent(nodeKey) → Groups_Group{id…}, 根 = 0) ----
    // shapeMaxFaces: 面数不超过它的零件才做回转面摘要(标准件识别; 复杂件不算, 省时间)
    bom: { relTol: 2e-3, chiralEps: 1e-3, groupId: 'id', rootGroupId: 0, shapeMaxFaces: 80 },
    // D 导出 STEP(实测 26.2.3, 2026-11-16): 站内对话框元素名、格式按钮文字、解析面转 NURBS 复选框名; OK / Cancel 在
    // 对话框外层容器里(往上找同时含两者的祖先, 最多 boxClimb 层); 选择里要记 / 还原的集合。
    exportStep: { dialogTag: 'export-step-58-dialog', formats: ['AP203', 'AP214', 'AP242'], defaultFormat: 'AP214',
                  nurbsInput: 'saveAnalyticsToNurbs', okText: 'OK', cancelText: 'Cancel', boxClimb: 6,
                  dialogWaitMs: 10000, closeWaitMs: 20000, selSets: ['solids', 'sheets', 'curves', 'faces', 'edges'],
                  // 选上实体时 Plasticity 自动起的移动命令(agent automatic); 还原成空选择后它还挂着 → 取消(实测 2026-11-18)
                  autoMove: { cmd: 'MoveItemCommand', agentKey: 'agent', agent: 'automatic' } },
    // ---- 右键确认扩展到 Plasticity 窗口(ui.rightClick): 捕获这些事件, 右键松开计一次; contextmenu 只吃不计 ----
    rightClick: { events: ['pointerdown', 'pointerup', 'mousedown', 'mouseup', 'contextmenu'], button: 2,
                  countOn: 'pointerup', menuEvent: 'contextmenu' },
    // ---- N3e: 本体测量(know-how/geometry-query.md) ----
    // 两点测量工厂: p1(点, 吸附对象) / p2(点, 吸附对象)(Measure Distance 实录); 量的方向 = orientation(四元数)把
    // axis 转到哪 —— 设成参考面法向, 标签值 = 两面法向距离(实测 16.02 = dim.face 候选值)。
    // 测量记录列表 = snapshot().<list>; 名字: nodes.getName / setName(nodes.item2key(测量记录))。
    measure: { list: 'repo.measurements', p1: 'p1', p2: 'p2', orientation: 'orientation', axis: [1, 0, 0],
               faceSnapClass: 'FaceCenterPointSnap', snapLookup: 'lookup', snapPos: 'position', lookupMax: 4000,
               snapTolM: 1e-5 }
  };

  // =====================================================================
  // 2. 能力表：needs = 必需表键；optional = 缺了只降级（探测里列为警告）；todo = 路线图预留。
  // =====================================================================
  var selKindKeys = SEL_KINDS.map(function (k) { return 'sel.ids.' + k[0]; });
  var orbitKeys = [];
  ORBIT_FIELDS.forEach(function (f) { orbitKeys.push('oc.' + f[0], 'oc.' + f[0] + 'End'); });

  var CAPS = {
    'editor':    { needs: [] },
    'sel.read':  { needs: ['sel.selected', 'sel.size'], optional: selKindKeys },
    'sel.write': { needs: ['exec.cancel'] },
    'sel.apply': { needs: ['ed.geo', 'geo.lookupTopo', 'geo.lookupItem', 'cs.onClick', 'ed.enqueue',
                           'cmd.selectionBaseHolder', 'sig.commandEnded', 'exec.cancel', 'sel.selected'],
                   optional: ['sel.add', 'sel.removeAll'] },  // 实体点面没选上时命令内直接加入(兜底)
    'hist.read': { needs: ['ed.undoStack', 'ed.geoVersion'] },
    'cam.read':  { needs: ['vp.camera'], optional: ['vp.orbit'].concat(orbitKeys) },
    'cam.write': { needs: ['vp.camera', 'vp.orbit', 'oc.quaternionEnd', 'oc.targetEnd', 'oc.radiusEnd', 'vp.render'] },
    'cam.nav':   { needs: ['vp.navigate'] },
    'cam.step':  { needs: ['oc.step'], optional: ['orbitSettings.step'] },
    'cam.roll':  { needs: ['reg.add', 'reg.listeners', 'oc.quaternionEnd', 'oc.step'], optional: ['oc.domElement'] },
    'vp.state':  { needs: ['vp.camera', 'vp.orbit', 'ed.shading.mode', 'ed.shading.changed', 'ed.sections'],
                   optional: ['ed.shading.section', 'vp.cplane', 'vp.settings.renderer', 'vp.settings.ao', 'vp.settings.grid', 'vp.container',
                              'vp.snapshot', 'vp.init', 'vp.isRenderMode', 'vp.toggleRenderMode', 'vp.togglePerspective', 'ed.layers', 'ed.docSettings',
                              'ed.originator', 'ed.shadingDb', 'nodes.state', 'sh.section', 'sh.isSectioned'] },
    'keymap':    { needs: ['km.add', 'km.bindings'], optional: ['kb.update'] },
    'cmd.dispatch': { needs: ['reg.add', 'reg.listeners'] },
    'err.hook':  { needs: ['ed.errorSignal'] },
    'tree':      { needs: [], dom: true },
    'mat.state': { needs: ['ed.originator', 'nodes.state'], optional: ['mat.textures', 'tex.get', 'mats.snapshot'] },
    // B1/B2 BOM: 实体 + 显示网格 + 稳定 id; 组路径 / 材质缺了只降级
    'outline.read': { needs: ['ed.groupsDb', 'groups.root', 'groups.children', 'groups.all', 'groups.byId', 'nodes.key2item', 'nodes.item2key', 'nodes.getName', 'nodes.isHidden', 'nodes.isVisible'], optional: ['nodes.isLocked'] },
    'bom.read': { needs: ['geo.items', 'geo.lookupTopo', 'nodes.state', 'nodes.item2key', 'nodes.getName', 'ed.docUnit'], optional: ['ed.groupsDb', 'groups.getParent', 'mats.snapshot', 'ed.originator'] },
    // D 导出 STEP: 选择增删 + 导出入口; 站内对话框是普通 DOM 元素, 运行时查(不用 dom 标记 —— 那个指 Preact 树可读, 且会跳过 needs)
    'file.export': { needs: ['ed.export', 'sel.selected', 'sel.add', 'sel.removeAll', 'geo.items', 'nodes.state', 'nodes.item2key'] },
    // C0 配置: 可见性 + 材质指派(与 vp.state 的可见性段、mat.state 同一批内部结构)
    'cfg.state': { needs: ['ed.originator', 'nodes.state', 'ed.nodes', 'nodes.item2key', 'nodes.getName', 'geo.items'], optional: ['mat.textures', 'tex.get', 'nodes.isolate', 'nodes.unisolate', 'mats.snapshot'] },
    'mat.set':     { todo: 'N1' },
    'geo.read':  { needs: ['geo.items', 'geo.lookupTopo', 'geo.lookupItem', 'ed.docUnit'] },
    'hist.write': { needs: ['ed.undoStack', 'exec.cancel'] },
    'snap.read': { needs: ['geo.items', 'ed.docUnit', 'ed.snapScene', 'snap.canonicals'], optional: ['snap.occurrences'] },
    'view.highlight': { needs: ['geo.items', 'geo.lookupTopo', 'ed.helpers', 'helpers.add', 'helpers.remove', 'gizmo.meshMaterial'] },
    // 工厂类不列入: 首次使用前本来就没有(由代理按需挂上), 缺哪个在执行时按表键报
    'cmd.exec':    { needs: ['ed.enqueue', 'sig.commandEnded', 'exec.cancel', 'cmd.factoryBaseHolder', 'geo.items',
                             'geo.lookupTopo', 'geo.lookupItem', 'ed.docUnit', 'vp.camera'] },
    'measure.write': { needs: ['ed.measurements', 'meas.snapshot', 'ed.nodes', 'nodes.item2key', 'nodes.getName', 'nodes.setName',
                               'ed.snapScene', 'snap.canonicals', 'ed.enqueue', 'sig.commandEnded', 'exec.cancel', 'cmd.factoryBaseHolder',
                               'geo.items', 'geo.lookupTopo', 'geo.lookupItem', 'ed.docUnit'] },
    'bool.run':    { todo: 'N4' },
    'asset.lib': { needs: ['ed.assets', 'assets.add', 'assets.remove', 'nodes.setName', 'nodes.setPivot', 'nodes.isAsset', 'nodes.findAllAssets', 'cmd.toggleAsset', 'cmd.exportAssetPack', 'ed.enqueue', 'ed.executor', 'nodes.item2key'] },
    'asset.place': { needs: ['ed.assets', 'assets.packs', 'assets.load', 'ed.executor', 'exec.last', 'geo.items', 'ed.undoStack', 'ed.nodes', 'nodes.item2key', 'nodes.getName'] }
  };

  // =====================================================================
  // 3. 解析器
  // =====================================================================
  var MISS = {};
  var state = { ed: null, finder: null, gen: 0, hooks: {}, lastError: null, hits: {}, pings: 0,
                selCmd: null, disposables: [], lastMaintain: 0, describeCache: new Map() };

  function errMsg(e) { return e && e.message ? String(e.message) : String(e); }

  // 沿点分路径取值；任何一段不存在 → MISS。末段只要求"属性存在"（值可以是 null）。
  function walk(obj, path) {
    var parts = path.split('.');
    var cur = obj;
    for (var i = 0; i < parts.length; i++) {
      if (cur === null || cur === undefined) return MISS;
      var t = typeof cur;
      if (t !== 'object' && t !== 'function') return MISS;
      var k = parts[i];
      try {
        if (!(k in cur)) return MISS;
        cur = cur[k];
      } catch (e) { return MISS; }
    }
    return cur;
  }

  function rootOf(name) {
    switch (name) {
      case 'ed': return editor();
      case 'sel': return get('sel.selected');
      case 'vp': return viewport();
      case 'oc': return get('vp.orbit');
      case 'reg': return get('ed.registry');
      case 'geo': return get('ed.geo');
      case 'nodes': return get('ed.nodes');
      case 'cs': return get('ed.changeSelection');
      case 'exec': return get('ed.executor');
      case 'km': return get('ed.keymaps');
      case 'kb': return get('ed.keyboard');
      case 'orbitSettings': return get('ed.settings.orbit');
      case 'orig': return get('ed.originator');
      case 'sh': return get('ed.shadingDb');
      case 'ds': return get('ed.docSettings');
      case 'tex': return get('mat.textures');
      case 'fac': return W[CFG.factoriesGlobal];
      case 'ss': return get('ed.snapScene');
      case 'helpers': return get('ed.helpers');
      case 'meas': return get('ed.measurements');
      case 'mats': return get('ed.materialsDb');
      case 'groupsDb': return get('ed.groupsDb');
      case 'scene': return get('ed.scene');
      case 'assets': return get('ed.assets');
    }
    return undefined;
  }

  // 解析表键：返回 {ok, v, path}。base 可显式给出根对象（如遍历每个视口时）。
  function resolve(key, base) {
    var spec = LOC[key];
    if (!spec) throw new Error('shim: unknown LOC key ' + key);
    var root = base !== undefined ? base : rootOf(spec[0]);
    if (root === null || root === undefined) return { ok: false };
    for (var i = 1; i < spec.length; i++) {
      var v = walk(root, spec[i]);
      if (v !== MISS) {
        if (state.hits[key] !== spec[i]) state.hits[key] = spec[i];
        return { ok: true, v: v, path: spec[i] };
      }
    }
    return { ok: false };
  }

  function get(key, base) { var r = resolve(key, base); return r.ok ? r.v : undefined; }

  function CapError(cap, missing) { this.cap = cap; this.missing = missing; this.message = 'cap:' + cap; }

  // 取必需表键；缺失抛 CapError（由 call() 转成 {ok:false, err:'cap:..'}）。
  function need(key, base) {
    var r = resolve(key, base);
    if (!r.ok) throw new CapError(key, [key]);
    return r.v;
  }

  // 写入表键对应的属性(按实际命中的那条路径写); 返回是否写成。
  function put(key, val, base) {
    var spec = LOC[key];
    var r = resolve(key, base);
    if (!r.ok) return false;
    var root = base !== undefined ? base : rootOf(spec[0]);
    var parts = r.path.split('.');
    var last = parts.pop();
    var parent = parts.length ? walk(root, parts.join('.')) : root;
    if (parent === MISS || parent === null || parent === undefined) return false;
    try { parent[last] = val; return true; } catch (e) { return false; }
  }

  // 向量类(three.js Vector3 / Quaternion)按分量写: 有 set 用 set。
  function setVec(obj, arr) {
    if (!obj || !arr || typeof obj.set !== 'function') return false;
    try { obj.set.apply(obj, arr); return true; } catch (e) { return false; }
  }

  // 调用方法类表键: this = 该表键所属的根对象(如 exec.cancel 的 this = ed.executor)。缺失抛 CapError。
  function invokeOn(key, args, base) {
    var spec = LOC[key];
    var owner = base !== undefined ? base : rootOf(spec[0]);
    var fn = need(key, base);
    if (typeof fn !== 'function') throw new CapError(key, [key]);
    return fn.apply(owner, args || []);
  }

  function call0(obj, key) {
    var f = get(key, obj);
    if (typeof f === 'function') { try { return f.call(obj); } catch (e) {} }
    return undefined;
  }

  // =====================================================================
  // 4. 编辑器与视口
  // =====================================================================
  function isEditor(e) {
    if (!e || typeof e !== 'object') return false;
    var sig = CFG.editorSignature || [];
    for (var i = 0; i < sig.length; i++) if (walk(e, sig[i]) === MISS || e[sig[i]] == null) return false;
    return sig.length > 0;
  }

  function domRoot() {
    var el = document.getElementById(NAMES.appRootId);
    return el && el[NAMES.vnodeChildren] ? el[NAMES.vnodeChildren] : null;
  }

  // 广度遍历 Preact vnode 树；visit 返回非 undefined 即停止并返回该值。
  function eachVnode(visit) {
    var start = domRoot();
    if (!start) return undefined;
    var q = [start], seen = new Set(), g = 0, kk = NAMES.vnodeChildren;
    while (q.length && g++ < NAMES.vnodeScanLimit) {
      var v = q.shift();
      if (!v || typeof v !== 'object' || seen.has(v)) continue;
      seen.add(v);
      var r = visit(v);
      if (r !== undefined) return r;
      var k = v[kk];
      if (Array.isArray(k)) q.push.apply(q, k);
      else if (k && typeof k === 'object') q.push(k);
    }
    return undefined;
  }

  var FINDERS = [
    ['global', function () { return W[CFG.editorGlobal]; }],
    ['preact', function () {
      return eachVnode(function (v) {
        if (typeof v.type === 'function' && v.type.name === NAMES.outlinerComponent &&
            v.props && v.props.model && v.props.model.editor) return v.props.model.editor;
        return undefined;
      });
    }]
  ];

  function editor() {
    if (state.ed && isEditor(state.ed) && W[CFG.editorGlobal] === state.ed) return state.ed;
    for (var i = 0; i < FINDERS.length; i++) {
      var e;
      try { e = FINDERS[i][1](); } catch (x) { e = null; }
      if (isEditor(e)) {
        if (state.ed !== e) { disposeHooks(); state.ed = e; state.gen++; }
        state.finder = FINDERS[i][0];
        W[CFG.editorGlobal] = e;
        return e;
      }
    }
    state.ed = null; state.finder = null;
    return null;
  }

  // 视口选择策略（唯一一处）：目前取第一个视口，与历史行为一致；多视口支持只改这里。
  function viewport() {
    var vps = get('ed.viewports');
    if (!vps) return null;
    var first = null;
    try { vps.forEach(function (v) { if (!first) first = v; }); } catch (e) {}
    return first;
  }

  // =====================================================================
  // 5. 能力探测（由 CAPS 自动生成）
  // =====================================================================
  function capState(name) {
    var c = CAPS[name];
    if (!c) return { ok: false, missing: ['unknown-cap:' + name] };
    if (c.todo) return { ok: false, todo: c.todo, missing: [] };
    if (name !== 'editor' && !c.dom && !editor()) return { ok: false, missing: ['editor'] };
    if (name === 'editor') return editor() ? { ok: true, missing: [] } : { ok: false, missing: ['editor'] };
    if (c.dom) return domRoot() ? { ok: true, missing: [] } : { ok: false, missing: ['dom:#' + NAMES.appRootId] };
    var missing = [], warn = [];
    (c.needs || []).forEach(function (k) { if (!resolve(k).ok) missing.push(k); });
    (c.optional || []).forEach(function (k) { if (!resolve(k).ok) warn.push(k); });
    var r = { ok: missing.length === 0, missing: missing };
    if (warn.length) r.optionalMissing = warn;
    return r;
  }

  function probe() {
    var caps = {};
    Object.keys(CAPS).forEach(function (n) { caps[n] = capState(n); });
    var hits = {};
    Object.keys(state.hits).forEach(function (k) {
      if (state.hits[k] !== LOC[k][1]) hits[k] = state.hits[k]; // 只报走了备选路径的键
    });
    // 环境诊断(不属于能力): 官方 Bridge 服务端状态 — bridge 连不上时用来说清原因。
    var env = {};
    if (editor()) {
      var bs = get('ed.bridge.state');
      env.bridge = {
        state: typeof bs === 'number' ? bs : null,
        port: get('ed.bridge.port') || null,
        enabled: get('ed.bridge.enabled') === true
      };
    }
    return {
      api: API_VERSION, rev: CFG.rev, plasticity: CFG.plasticityVersion || null,
      editor: { ok: !!editor(), finder: state.finder, gen: state.gen },
      caps: caps, altPaths: hits, env: env
    };
  }

  // =====================================================================
  // 6. 常驻钩子（按 editor 代际安装；editor 变化后自动重装；dispose 全部拆除）
  // =====================================================================
  var HOOKS = [
    ['err', 'err.hook', function () {
      var sig = need('ed.errorSignal');
      var fn = function (p) {
        try {
          if (!p) return;
          if (!((p instanceof Error) || (p.message && p.stack))) return;
          state.lastError = { msg: String(p.message || p).slice(0, 200), stack: String(p.stack || '').slice(0, 800), ts: Date.now() };
        } catch (e) {}
      };
      var binding = sig.add(fn);
      return function () {
        try { if (binding && typeof binding.detach === 'function') binding.detach(); else sig.remove(fn); } catch (e) {}
      };
    }]
  ];

  function ensureHooks() {
    if (!editor()) return;
    HOOKS.forEach(function (h) {
      if (state.hooks[h[0]]) return;
      if (!capState(h[1]).ok) return;
      try { state.hooks[h[0]] = h[2]() || function () {}; } catch (e) {}
    });
    // 常驻维护(节流): 视口元素可能重建, 滚转命令要补注册。
    var now = Date.now();
    if (now - state.lastMaintain >= NAMES.maintainEveryMs) {
      state.lastMaintain = now;
      if (capState('cam.roll').ok) { try { maintainRollCommands(); } catch (e) {} }
    }
  }

  function disposeHooks() {
    Object.keys(state.hooks).forEach(function (k) { try { state.hooks[k](); } catch (e) {} });
    state.hooks = {};
    state.disposables.forEach(function (d) { try { d.dispose(); } catch (e) {} });
    state.disposables = [];
  }

  // =====================================================================
  // 7. 接口表（对外唯一入口；名字与 Rust shim::api 一一对应）
  // =====================================================================
  var API = {};
  // opts.async: 接口返回 Promise(要等命令执行完); 这类接口不许进 batch(会产生副作用却拿不到结果)。
  function def(name, cap, fn, opts) { API[name] = { cap: cap, fn: fn, async: !!(opts && opts.async) }; }

  function v3(v) { return v && typeof v.x === 'number' ? [v.x, v.y, v.z] : null; }

  // 选择集元素保真：数值/字符串原样；对象形（face/edge 的 {id,versionId,...}）抽标量属性 + __ctor + __str。
  function idElem(x) {
    if (x === null || x === undefined) return null;
    if (typeof x !== 'object') return x;
    var c = { __ctor: (x.constructor && x.constructor.name) || '' };
    try { Object.keys(x).forEach(function (k) { var val = x[k]; if (val === null || typeof val !== 'object') c[k] = val; }); } catch (e) {}
    try { c.__str = String(x); } catch (e2) {}
    return c;
  }

  def('core.probe', null, function () { return probe(); });
  def('core.health', null, function () {
    return { api: API_VERSION, rev: CFG.rev, editor: !!editor(), finder: state.finder, hooks: Object.keys(state.hooks) };
  });
  def('core.apis', null, function () { return Object.keys(API); });

  def('sel.snapshot', 'sel.read', function () {
    var s = need('sel.selected');
    var items = [];
    SEL_KINDS.forEach(function (k) {
      var set = get('sel.ids.' + k[0], s);
      if (!set || typeof set.forEach !== 'function') return;
      set.forEach(function (x) {
        var vk = NAMES.versionIdKey;
        var id = String(x && typeof x === 'object' && x[vk] !== undefined ? x[vk] : x);
        // info: 可读描述(所属物体名 / 自身名字 / 曲面类型), 供界面区分同类项。
        items.push({ kind: k[0], e: idElem(x), info: describe(k[0], id) });
      });
    });
    var size = get('sel.size', s);
    // rclick: 助手"右键确认"在 Plasticity 窗口里被点的次数(见 ui.rightClick; 随轮询带回, 助手看到增加 = 确认)
    return { size: typeof size === 'number' ? size : -1, items: items, rclick: state.rc ? state.rc.count : 0 };
  });

  // 右键确认扩展到整个 Plasticity 窗口(用户 2026-11-15)。助手有等待确认的操作时 arm = true:
  // 页面里的右键(捕获阶段)被吃掉 —— 不弹 Plasticity 菜单、不结束它的命令 —— 只记一次数; arm = false 时完全不碰右键。
  // args = { arm: bool } → { armed, count }
  def('ui.rightClick', null, function (a) {
    var R = NAMES.rightClick;
    if (!state.rc) {
      var rc = state.rc = { armed: false, count: 0 };
      var h = function (e) {
        if (!rc.armed || (e.type !== R.menuEvent && e.button !== R.button)) return;
        e.preventDefault(); e.stopImmediatePropagation();
        if (e.type === R.countOn) rc.count++;
      };
      R.events.forEach(function (t) { W.addEventListener(t, h, true); });
      state.disposables.push({ dispose: function () { R.events.forEach(function (t) { W.removeEventListener(t, h, true); }); state.rc = null; } });
    }
    if (a && a.arm !== undefined) state.rc.armed = !!a.arm;
    return { armed: state.rc.armed, count: state.rc.count };
  });

  def('hist.state', 'hist.read', function () {
    var st = need('ed.undoStack');
    var gv = need('ed.geoVersion');
    return { undo: st && typeof st.length === 'number' ? st.length : -1, geoVer: typeof gv === 'number' ? gv : -1 };
  });

  // 相机：position/quaternion + 透视 zoom/fov + 轨道控制器全部阻尼量。绝不读写投影矩阵（R7-31h）。
  def('cam.get', 'cam.read', function () {
    var cam = need('vp.camera');
    var o = { px: cam.position.x, py: cam.position.y, pz: cam.position.z,
              qx: cam.quaternion.x, qy: cam.quaternion.y, qz: cam.quaternion.z, qw: cam.quaternion.w };
    if (typeof cam.zoom === 'number') o.zoom = cam.zoom;
    if (typeof cam.fov === 'number') o.fov = cam.fov;
    var oc = get('vp.orbit');
    if (oc) {
      var orb = {};
      ORBIT_FIELDS.forEach(function (f) {
        [f[0], f[0] + 'End'].forEach(function (name) {
          var val = get('oc.' + name, oc);
          if (f[3] === 'v3') { val = v3(val); if (val) orb[name] = val; }
          else if (typeof val === 'number') orb[name] = val;
        });
      });
      o.orbit = orb;
    }
    return o;
  });

  // 切到给定视角(参数 = cam.get 的输出; 老存档可缺 orbit/zoom/fov)。
  // 顺序: 先写相机, 再写轨道控制器(当前值 + *End 两份, 否则阻尼插值会拉回), 最后 update + 重绘。
  // 投影参数只在透视相机上写(正交 ProxyCamera 的 updateProjectionMatrix 有破坏性副作用, R7-31h)。
  def('cam.set', 'cam.write', function (o) {
    var cam = need('vp.camera');
    cam.position.set(o.px, o.py, o.pz);
    cam.quaternion.set(o.qx, o.qy, o.qz, o.qw);
    var persp = cam[NAMES.perspectiveFlag] === true;
    var proj = false;
    if (persp) {
      if (typeof o.zoom === 'number' && typeof cam.zoom === 'number') { cam.zoom = o.zoom; proj = true; }
      if (typeof o.fov === 'number' && typeof cam.fov === 'number') { cam.fov = o.fov; proj = true; }
    }
    var oc = get('vp.orbit');
    var orbit = false;
    if (o.orbit && oc) {
      ORBIT_FIELDS.forEach(function (f) {
        var cur = o.orbit[f[0]];
        var end = o.orbit[f[0] + 'End'];
        if (end === undefined || end === null) end = cur;
        if (cur === undefined || cur === null) return;
        if (f[3] === 'v3') {
          setVec(get('oc.' + f[0], oc), cur);
          setVec(get('oc.' + f[0] + 'End', oc), end);
        } else {
          put('oc.' + f[0], cur, oc);
          put('oc.' + f[0] + 'End', end, oc);
        }
      });
      // 朝向也同步给轨道控制器(否则下一次拖动会从旧朝向跳回来)。
      setVec(get('oc.quaternion', oc), [o.qx, o.qy, o.qz, o.qw]);
      setVec(get('oc.quaternionEnd', oc), [o.qx, o.qy, o.qz, o.qw]);
      orbit = true;
    }
    if (persp && proj) { try { cam.updateProjectionMatrix(); } catch (e) {} }
    call0(oc, 'oc.update');
    // 标记轨道控制器"需要更新": 否则它的更新循环要等下一次交互才跑(R21 用户反馈: 恢复后缩放要动一下视口才生效)。
    markOrbitDirty(oc);
    call0(viewport(), 'vp.render');
    return { proj: proj, orbit: orbit };
  });

  function markOrbitDirty(oc) {
    if (!oc) return;
    try { invokeOn('oc.setNeedsUpdate', [], oc); return; } catch (e) {}
    put('oc.needsUpdate', 1, oc);
  }

  // ---- 命令派发: 对视口元素派发 CustomEvent(Plasticity CommandRegistry 的执行方式) ----
  function viewportElement() {
    try { return document.querySelector(CFG.viewportSelector); } catch (e) { return null; }
  }

  function hasListener(name, el) {
    var m = need('reg.listeners');
    var w = m && (typeof m.get === 'function' ? m.get(name) : m[name]);
    return !!(w && typeof w.has === 'function' && w.has(el));
  }

  function dispatchOn(el, name) {
    el.dispatchEvent(new CustomEvent(name, { bubbles: true }));
  }

  def('cmd.dispatch', 'cmd.dispatch', function (a) {
    var el = viewportElement();
    if (!el) throw new Error('no viewport element (' + CFG.viewportSelector + ')');
    if (!a || typeof a.name !== 'string') throw new Error('cmd.dispatch needs {name}');
    dispatchOn(el, a.name);
    return true;
  });

  // ---- 注册到 Plasticity 的命令一律"委托": 回调触发时才去找**当前**的 window.__PLA__ ----
  // 回调若闭包捕获某个垫片实例, 热替换后它仍指向旧实例(R21 实测: 旧实例的 ping 回调让新实例计数不动)。
  // 哪些元素已挂委托记在页面级唯一的 __PLA_PAGE__ 上(跨实例存续), 热替换不会重复注册。
  function pageState() {
    var p = W.__PLA_PAGE__;
    // delegated: 命令注册表 → { 命令名: WeakSet(已挂委托的元素) }(editor 换代 = 新注册表, 要重新挂)。
    if (!p) { p = { delegated: new WeakMap() }; W.__PLA_PAGE__ = p; }
    return p;
  }

  function ensureDelegate(cmdName, el, action, force) {
    var page = pageState();
    var reg = need('ed.registry');
    var byName = page.delegated.get(reg);
    if (!byName) { byName = {}; page.delegated.set(reg, byName); }
    var set = byName[cmdName] || (byName[cmdName] = new WeakSet());
    // 页面级记录说挂过, 还要注册表里确实还在: 热替换时旧实例 dispose 会注销它挂的监听, 记录却跨实例保留
    // → 以前就此认定"已挂"再不补挂, ping / 滚转命令失灵到页面重载(2026-11-15 自检实测)。
    if (set.has(el) && hasListener(cmdName, el)) return false;
    // 已有别处注册的同名回调(旧版助手留下的)且不强制: 不再叠加, 避免一次按键执行两次。
    if (!force && hasListener(cmdName, el)) return false;
    var map = {};
    map[cmdName] = function () {
      var cur = W.__PLA__;
      if (cur && typeof cur._onCommand === 'function') cur._onCommand(action, el);
    };
    // CommandRegistry.add 只有对象形式 add(target, {名: 回调})(pitfalls #21)。
    var d = invokeOn('reg.add', [el, map]);
    if (d && typeof d.dispose === 'function') state.disposables.push(d);
    set.add(el);
    return true;
  }

  // 委托回调的落点(由当前实例执行)。
  function onCommand(action, el) {
    if (action === 'ping') state.pings++;
    else if (action === 'rollLeft') rollBy(viewportOfElement(el), -1);
    else if (action === 'rollRight') rollBy(viewportOfElement(el), 1);
  }

  // 自检用: 在视口元素上注册一个空命令(幂等), 派发它, 确认回调真的执行了。
  // 覆盖"注册(对象形式 add)→ 派发 → 回调"整条链(滚转、命令派发层都依赖它)。
  def('cmd.ping', 'cmd.dispatch', function () {
    var el = viewportElement();
    if (!el) throw new Error('no viewport element (' + CFG.viewportSelector + ')');
    var name = CFG.pingCommand;
    ensureDelegate(name, el, 'ping', false);
    var before = state.pings;
    dispatchOn(el, name);
    if (state.pings === before) {
      // 只有旧实例留下的(闭包式)回调: 强制补挂委托(ping 是空操作, 叠加无害)再试一次。
      ensureDelegate(name, el, 'ping', true);
      dispatchOn(el, name);
    }
    return { registered: hasListener(name, el), fired: state.pings > before };
  });

  // ---- 选择写入: 一律经 executor 执行选择命令(与手点同一条链路; selection-by-command.md) ----
  // 不再用 click.addById(不产生命令、Move 一接手就丢选择, 还会写畸形条目)和屏幕扫描点击(漏面、受遮挡/视角/节流影响)。
  function clearBusyAttrs() {
    NAMES.bodyBusyAttrs.forEach(function (a) { try { document.body.removeAttribute(a); } catch (e) {} });
  }

  def('sel.cancelCommand', 'sel.write', function () {
    invokeOn('exec.cancel');
    clearBusyAttrs();
    return true;
  });

  function idList(s, kind) {
    var out = [];
    var set = get('sel.ids.' + kind, s);
    var vk = NAMES.versionIdKey;
    if (set && typeof set.forEach === 'function') {
      set.forEach(function (x) { out.push(String(x && typeof x === 'object' && x[vk] !== undefined ? x[vk] : x)); });
    }
    return out;
  }

  // 按 (kind, id) 查视图对象: 拓扑项(面/边…)走 lookupTopologyItemById, 物体(实体/片体/曲线…)走 lookupItemById。
  function firstView(v) { return v && typeof v.values === 'function' && !v.isObject3D ? v.values().next().value : v; }

  // 几何库查找结果 { view, model, … }; 查不到 → null。
  function lookupOf(kind, id) {
    var t = null;
    if (NAMES.topologyKinds.indexOf(kind) >= 0) {
      try { t = invokeOn('geo.lookupTopo', [String(id)]); } catch (e) { t = null; }
    } else {
      var n = Number(id);
      try { t = invokeOn('geo.lookupItem', [isNaN(n) ? id : n]); } catch (e) { t = null; }
      if (!t && !isNaN(n)) { try { t = invokeOn('geo.lookupItem', [String(id)]); } catch (e2) { t = null; } }
    }
    return t || null;
  }

  function viewOf(kind, id) {
    var t = lookupOf(kind, id);
    return t ? firstView(t[NAMES.lookupViewKey]) : null;
  }

  // 物体在大纲里的名字(实体/片体/曲线…)。
  function nameOf(view) {
    if (!view) return null;
    try {
      var n = invokeOn('nodes.getName', [invokeOn('nodes.item2key', [view])]);
      return typeof n === 'string' && n ? n : null;
    } catch (e) { return null; }
  }

  // 选择项的可读描述(界面区分同类项用): 拓扑项 → 所属物体名 + 曲面类型(面); 物体 → 自身名字。
  // 按 kind:版本 id 缓存 — 几何一变版本 id 就变, 缓存不会过期; 超上限整表清空。
  function describe(kind, id) {
    var key = kind + ':' + id;
    var hit = state.describeCache.get(key);
    if (hit) return hit;
    var info = {};
    var t = lookupOf(kind, id);
    var view = t ? firstView(t[NAMES.lookupViewKey]) : null;
    if (view) {
      if (NAMES.topologyKinds.indexOf(kind) >= 0) {
        var owner = view[NAMES.parentItemKey];
        var on = nameOf(owner);
        if (on) info.ownerName = on;
        try {
          var model = t[NAMES.lookupModelKey];
          var s = model && typeof model[NAMES.surfaceGetter] === 'function' ? model[NAMES.surfaceGetter]() : null;
          var inner = s && s[NAMES.surfaceInnerKey];
          if (inner && inner.constructor && inner.constructor.name) info.surface = inner.constructor.name;
        } catch (e) {}
      } else {
        var n = nameOf(view);
        if (n) info.name = n;
      }
    }
    if (state.describeCache.size >= NAMES.describeCacheLimit) state.describeCache.clear();
    state.describeCache.set(key, info);
    return info;
  }

  // 构造一个"命中"(与真实拾取结果同构, 实测 {distance, point, intersectable, object, distance2d})。
  function hitFor(view) {
    var point = null;
    try {
      var box = view.getBoundingBox();
      point = new box.min.constructor();
      box.getCenter(point);
    } catch (e) {
      var cam = get('vp.camera');
      point = cam ? new cam.position.constructor() : { x: 0, y: 0, z: 0 };
    }
    return { distance: 1, point: point, intersectable: view, object: view, distance2d: 0 };
  }

  // 实体 / 片体的第一个面的版本 id(面组 versionIds); 没有 → null
  function firstFaceId(view) {
    var g = view && view[NAMES.shellGroups.face];
    var ids = g && g[NAMES.groupVersionIds];
    if (!ids || !ids.length) return null;
    return String(ids[0]);
  }

  // 选择命令: 继承 Plasticity 的 SelectionCommand(从已注册的选择类命令原型链取基类, 原生点击命令类只在闭包里)。
  // execute 里逐项走 changeSelection.onClick(与真实点击同一处理), 整个选择集只占一个撤销步。
  function selectionCommandClass() {
    var Ctor = need('cmd.selectionBaseHolder');
    var Base = Object.getPrototypeOf(Ctor.prototype).constructor;
    if (state.selCmd && state.selCmd.base === Base) return state.selCmd.cls;
    class PlaApplySelection extends Base {
      constructor(editor, entries, replace) { super(editor); this.plaEntries = entries; this.plaReplace = replace; this.plaReport = null; }
      async execute() {
        var cs = need('ed.changeSelection');
        var onClick = need('cs.onClick');
        var sel = need('sel.selected');
        var drills = 0;
        var self = this;
        var direct = 0;
        this.plaEntries.forEach(function (en, i) {
          var mod = i === 0 && self.plaReplace ? NAMES.modReplace : NAMES.modAdd;
          if (NAMES.topologyKinds.indexOf(en.kind) < 0 && en.kind !== 'curve') {
            // 实体 / 片体: 拾取只会命中面, 拿实体对象本身当命中物点下去什么也不选(全是实体的选择集还原失败, 用户 2026-11-16)。
            // 改为点它的第一个面: 点未介入实体上的面 = 选中整个实体(与手点一致)。仍没选上(如该实体已介入) → 命令内直接加入。
            var fid = firstFaceId(en.view), fv = fid !== null ? viewOf('face', fid) : null;
            if (fv) onClick.call(cs, [hitFor(fv)], mod, 0);
            if (idList(sel, en.kind).indexOf(String(en.id)) < 0) {
              if (mod === NAMES.modReplace && !fv) invokeOn('sel.removeAll', [], sel);
              invokeOn('sel.add', [en.view], sel);
              direct++;
            }
            return;
          }
          var hit = hitFor(en.view);
          onClick.call(cs, [hit], mod, 0);
          // 点击语义: 未介入实体上的面可能先选中整个实体 → 同一命令内再点一次下钻。
          if (idList(sel, en.kind).indexOf(String(en.id)) < 0) { drills++; onClick.call(cs, [hit], NAMES.modAdd, 0); }
        });
        this.plaReport = { drills: drills, direct: direct };
      }
    }
    state.selCmd = { base: Base, cls: PlaApplySelection };
    return PlaApplySelection;
  }

  // 入队一个命令并等它结束。editor.enqueue 的 Promise 要等整条命令链结束(后续命令等输入就永不兑现) →
  // 改为监听 commandEnded 认本命令; 入队前先取消活动命令(否则新命令排队不执行)。
  // 启动看门狗(N4 实测): 紧跟在上一条命令之后入队, 偶尔整条命令不开始执行(commandEnded 永不来) →
  // 带 plaStarted 标记的命令(自建工厂命令)若 NAMES.cmdStartWatchMs 内没开始, 取消活动命令后重新入队(最多 NAMES.cmdStartRetries 次)。
  function runCommand(cmd, timeoutMs) {
    var sig = need('sig.commandEnded');
    try { invokeOn('exec.cancel'); } catch (e) {}
    return new Promise(function (resolve) {
      var done = false;
      var timer = null, watch = null, retries = 0;
      var finish = function (how) { if (done) return; done = true; sig.remove(f); if (timer) clearTimeout(timer); if (watch) clearTimeout(watch); resolve(how); };
      var f = function (c) { if (c === cmd) finish('ended'); };
      sig.add(f);
      timer = setTimeout(function () { finish('timeout'); }, timeoutMs || 10000);
      var arm = function () {
        if (cmd.plaStarted !== false) return;
        watch = setTimeout(function () {
          if (done || cmd.plaStarted !== false || retries >= NAMES.cmdStartRetries) return;
          retries++;
          try { invokeOn('exec.cancel'); } catch (e) {}
          invokeOn('ed.enqueue', [cmd]);
          arm();
        }, NAMES.cmdStartWatchMs);
      };
      invokeOn('ed.enqueue', [cmd]);
      arm();
    });
  }

  // 恢复一组选择(选择集应用 / 自检回环)。args = { items: [{kind, id}], replace: 是否替换当前选择(默认 true) }。
  // 返回 { selected, missing(查不到对象或没选上), unsupported(类型不支持), extra(多选的), drills, how }。
  def('sel.apply', 'sel.apply', function (a) {
    if (!a || !Array.isArray(a.items)) throw new Error('sel.apply needs {items: [{kind, id}]}');
    var replace = a.replace !== false;
    var entries = [], missing = [], unsupported = [];
    a.items.forEach(function (it) {
      var kind = String(it.kind), id = String(it.id);
      if (NAMES.applyKinds.indexOf(kind) < 0) { unsupported.push(kind + ':' + id); return; }
      var view = viewOf(kind, id);
      if (view) entries.push({ kind: kind, id: id, view: view }); else missing.push(kind + ':' + id);
    });
    if (!entries.length) return { selected: 0, missing: missing, unsupported: unsupported, extra: [], drills: 0, how: 'nothing' };
    var Cls = selectionCommandClass();
    var cmd = new Cls(editor(), entries, replace);
    return runCommand(cmd, 10000).then(function (how) {
      var sel = need('sel.selected');
      var wanted = {};
      entries.forEach(function (en) { wanted[en.kind + ':' + en.id] = true; });
      var have = {};
      NAMES.applyKinds.forEach(function (k) { idList(sel, k).forEach(function (id) { have[k + ':' + id] = true; }); });
      entries.forEach(function (en) { if (!have[en.kind + ':' + en.id]) missing.push(en.kind + ':' + en.id); });
      var extra = replace ? Object.keys(have).filter(function (k) { return !wanted[k]; }) : [];
      return { selected: entries.length - (missing.length - (a.items.length - entries.length - unsupported.length)),
               missing: missing, unsupported: unsupported, extra: extra, drills: cmd.plaReport ? cmd.plaReport.drills : 0, how: how };
    });
  }, { async: true });

  // 清空选择: 执行 Plasticity 自己的"全部取消选择"命令(click.clear 实测并不清空)。
  def('sel.clear', 'sel.apply', function () {
    var Ctor = need('cmd.selectionBaseHolder');
    return runCommand(new Ctor(editor()), 5000).then(function (how) { return { how: how, size: get('sel.size') }; });
  }, { async: true });

  // =====================================================================
  // N2: 命令派发(know-how/command-dispatch.md)
  // 不模拟点选: 每步 = 配方表里的几何工厂(new F(editor) → setter 设参 → commit), 包在自建 Command 里入队,
  // 与手工操作同路径(进撤销栈, 一步一个撤销步)。工厂类只在闭包里, 由代理堆快照挂到 CFG.factoriesGlobal;
  // 还没挂上时抛 CFG.needFactoriesErr, Rust 侧据此让代理挂一次再重试。
  // =====================================================================
  function factoryCommandClass() {
    var Base = Object.getPrototypeOf(need('cmd.factoryBaseHolder'));
    if (typeof Base !== 'function') throw new CapError('cmd.factoryBaseHolder', ['cmd.factoryBaseHolder']);
    if (state.facCmd && state.facCmd.base === Base) return state.facCmd.cls;
    class PlaFactoryCommand extends Base {
      constructor(editor, build) { super(editor); this.plaBuild = build; this.plaResult = null; this.plaErr = null; this.plaStarted = false; }
      async execute() {
        // 看门狗重新入队过的命令可能被执行第二次: 只执行一次
        if (this.plaStarted) return;
        this.plaStarted = true;
        // 失败要再抛出去: 让 Plasticity 按命令失败收尾(工厂随 resource 释放、不进撤销栈)
        try { this.plaResult = await this.plaBuild(this); } catch (e) { this.plaErr = errMsg(e); throw e; }
      }
    }
    state.facCmd = { base: Base, cls: PlaFactoryCommand };
    return PlaFactoryCommand;
  }

  function unitOf(a) {
    var u = a && a.units ? String(a.units) : get('ed.docUnit');
    var k = NAMES.unitScale[u];
    if (typeof k !== 'number') throw new Error('unknown unit: ' + u + ' (known: ' + Object.keys(NAMES.unitScale).join(', ') + ')');
    return { unit: u, k: k };
  }

  function solidViews() {
    var out = [];
    Array.from(need('geo.items') || []).forEach(function (it) {
      var v = it && it[NAMES.lookupViewKey];
      if (v && v.constructor && v.constructor.name === NAMES.solidCtor) out.push(v);
    });
    return out;
  }

  function itemIdOf(view) {
    var ud = view && view[NAMES.itemUserData];
    return ud && ud[NAMES.versionIdKey] !== undefined ? ud[NAMES.versionIdKey] : null;
  }

  // 物体包围盒(文档单位, 由逐面 bboxes 合成); 读不到 → null
  function bboxOf(view, k) {
    var g = view && view[NAMES.shellGroups.face];
    var b = g && g[NAMES.groupBboxes];
    if (!b || !b.length) return null;
    var mn = [Infinity, Infinity, Infinity], mx = [-Infinity, -Infinity, -Infinity];
    for (var i = 0; i + 5 < b.length; i += 6) {
      for (var j = 0; j < 3; j++) { mn[j] = Math.min(mn[j], b[i + j]); mx[j] = Math.max(mx[j], b[i + 3 + j]); }
    }
    // bboxes 是 Float32: 只保留 7 位有效数字(再多就是单精度噪声, 如 100.000002)
    var r = function (x) { return Number((x / k).toPrecision(7)); };
    return [mn.map(r), mx.map(r)];
  }

  function describeMade(view, k) {
    if (view && view.constructor && view.constructor.name === NAMES.outline.groupCtor) {
      var nm = null; try { nm = invokeOn('nodes.getName', [invokeOn('nodes.item2key', [view])]); } catch (e) {}
      return { kind: 'folder', id: view[NAMES.bom.groupId], name: nm };
    }
    var id = itemIdOf(view);
    if (id === null) return { kind: (view && view.constructor && view.constructor.name) || typeof view };
    return { kind: 'solid', id: id, name: nameOf(view), bbox: bboxOf(view, k) };
  }

  // 物体引用: '$名' = 前面某步 id 的产物(必须还活着) / {name} = 大纲名 / 其余 = 物体 id
  function resolveSolids(ref, refs) {
    if (typeof ref === 'string' && ref.charAt(0) === '$') {
      var made = refs[ref.slice(1)];
      if (!made) throw new Error('unknown ref ' + ref);
      var alive = solidViews();
      var v = made.filter(function (x) { return alive.indexOf(x) >= 0; });
      if (!v.length) throw new Error('ref ' + ref + ' no longer exists (consumed by a later step)');
      return v;
    }
    if (ref && typeof ref === 'object' && typeof ref.name === 'string') {
      var m = solidViews().filter(function (x) { return nameOf(x) === ref.name; });
      if (!m.length) throw new Error('no solid named ' + JSON.stringify(ref.name));
      return m;
    }
    var view = viewOf('solid', ref);
    if (!view) throw new Error('solid not found: ' + JSON.stringify(ref));
    return [view];
  }

  // 边/面选择: 拓扑 id 字符串, 或 {near:[x,y,z](文档单位), of?: 物体引用, tol?: 容差}。
  // 按逐项包围盒中心取最近的一项; 超出容差、或有两项一样近(分不清)都报错, 不猜。
  function pickTopo(kind, sel, ctx) {
    if (typeof sel === 'string' && sel.charAt(0) !== '$') {
      var byId = viewOf(kind, sel);
      if (!byId) throw new Error(kind + ' not found: ' + sel);
      return byId;
    }
    if (!sel || !Array.isArray(sel.near) || sel.near.length !== 3) throw new Error(kind + ' selector needs an id or {near:[x,y,z], of?, tol?}');
    var k = ctx.k;
    var pool = sel.of !== undefined ? resolveSolids(sel.of, ctx.refs) : solidViews();
    var p = sel.near.map(function (x) { return Number(x) * k; });
    var tol = (sel.tol !== undefined ? Number(sel.tol) : NAMES.pickTolerance) * k;
    var best = null, bd = Infinity, second = Infinity;
    pool.forEach(function (sv) {
      var g = sv[NAMES.shellGroups[kind]];
      var ids = g && g[NAMES.groupVersionIds], bx = g && g[NAMES.groupBboxes];
      if (!ids || !bx) return;
      ids = Array.from(ids);
      for (var i = 0; i < ids.length; i++) {
        var o = i * 6;
        var d = Math.hypot((bx[o] + bx[o + 3]) / 2 - p[0], (bx[o + 1] + bx[o + 4]) / 2 - p[1], (bx[o + 2] + bx[o + 5]) / 2 - p[2]);
        if (d < bd) { second = bd; bd = d; best = ids[i]; } else if (d < second) second = d;
      }
    });
    var show = function (x) { return Math.round(x / k * 1000) / 1000; };
    if (best === null || bd > tol) throw new Error(kind + ' near [' + sel.near.join(',') + ']: none within ' + show(tol) + (best === null ? '' : ' (nearest ' + show(bd) + ')'));
    if (second - bd < 1e-9 * Math.max(1, k)) throw new Error(kind + ' near [' + sel.near.join(',') + ']: ambiguous (two equally near)');
    var view = viewOf(kind, best);
    if (!view) throw new Error(kind + ' ' + best + ' could not be looked up');
    return view;
  }

  function convertParam(type, val, ctx) {
    var num = function (x) { var n = Number(x); if (!isFinite(n)) throw new Error('not a number: ' + JSON.stringify(x)); return n; };
    var list = function (x) { return Array.isArray(x) ? x : [x]; };
    switch (type) {
      case 'point':
        if (!Array.isArray(val) || val.length !== 3) throw new Error('point must be [x,y,z]');
        return new ctx.V3(num(val[0]) * ctx.k, num(val[1]) * ctx.k, num(val[2]) * ctx.k);
      case 'length': return num(val) * ctx.k;
      case 'angle': return num(val) * Math.PI / 180;
      case 'number': return num(val);
      case 'bool': return !!val;
      case 'edges': return list(val).map(function (s) { return pickTopo('edge', s, ctx); });
      case 'faces': return list(val).map(function (s) { return pickTopo('face', s, ctx); });
      case 'solids': return list(val).reduce(function (acc, r) { return acc.concat(resolveSolids(r, ctx.refs)); }, []);
      case 'translation':
        if (!Array.isArray(val) || val.length !== 3) throw new Error('move must be [dx,dy,dz]');
        return [num(val[0]) * ctx.k, num(val[1]) * ctx.k, num(val[2]) * ctx.k];
      // N4: 工厂实例上已有的 Vector3, 原地 set(米)
      case 'vecset':
        if (!Array.isArray(val) || val.length !== 3) throw new Error('point must be [x,y,z]');
        return [num(val[0]) * ctx.k, num(val[1]) * ctx.k, num(val[2]) * ctx.k];
      // N4: 方向 → 把工厂的基准轴(NAMES.cylinderFields.baseAxis)转到该方向的四元数(原地 setFromUnitVectors)
      case 'axisQuat': {
        if (!Array.isArray(val) || val.length !== 3) throw new Error('axis must be [x,y,z]');
        var l = Math.hypot(num(val[0]), num(val[1]), num(val[2]));
        if (!(l > 0)) throw new Error('axis must be non-zero');
        return [val[0] / l, val[1] / l, val[2] / l];
      }
      // N4: 倒角 = 圆角距离取负(Plasticity 约定; 实测出 45° 圆锥面)
      case 'negLength': return -Math.abs(num(val)) * ctx.k;
    }
    throw new Error('recipe: unknown param type ' + type);
  }

  // ---- A4: 草图类配方(多边形柱 / 旋转体): 画辅助曲线 → 取它生成的区域 → 拉伸 / 旋转(+ 布尔); 辅助曲线保留 ----
  function perpOf(ax) {
    var pp = Math.abs(ax[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0];
    var dp = vdot(pp, ax);
    return vnorm([pp[0] - dp * ax[0], pp[1] - dp * ax[1], pp[2] - dp * ax[2]]);
  }
  // 全部草图区域: [{id, center(米)}](与 versionIds 同序的 bboxes, 每项 6 个数)
  function allRegions() {
    var SF = NAMES.sketchFields, out = [];
    Array.from(need('geo.items') || []).forEach(function (it) {
      var v = it && it[NAMES.lookupViewKey];
      if (!v || !v.constructor || v.constructor.name !== SF.islandCtor) return;
      var g = v[SF.islandRegions], ids = g && g[NAMES.groupVersionIds], bx = g && g[NAMES.groupBboxes];
      if (!ids || !bx) return;
      Array.from(ids).forEach(function (id, i) {
        var o = i * 6;
        out.push({ id: String(id), center: [(bx[o] + bx[o + 3]) / 2, (bx[o + 1] + bx[o + 4]) / 2, (bx[o + 2] + bx[o + 5]) / 2] });
      });
    });
    return out;
  }
  // 提交辅助曲线后新出现的区域里, 中心离 expect 最近的那个(没有新区域 = 曲线并进了已有的草图 → 全部区域里找)
  function pickNewRegion(before, expect, k) {
    var all = allRegions();
    var fresh = all.filter(function (r) { return before.indexOf(r.id) < 0; });
    var pool = fresh.length ? fresh : all;
    var best = null, bd = Infinity;
    pool.forEach(function (r) { var d = Math.hypot(r.center[0] - expect[0], r.center[1] - expect[1], r.center[2] - expect[2]); if (d < bd) { bd = d; best = r; } });
    if (!best) throw new Error('the sketch curve made no closed region');
    if (!fresh.length && bd > NAMES.pickTolerance * k) throw new Error('no new region near the sketch (nearest ' + Math.round(bd / k * 1000) / 1000 + ')');
    var t = invokeOn('geo.lookupTopo', [best.id]);
    var view = t && t[NAMES.lookupViewKey];
    if (!view) throw new Error('region ' + best.id + ' could not be looked up');
    return view;
  }
  function customBuild(o, rc, ctx) {
    var k = ctx.k, V = ctx.V3, SF = NAMES.sketchFields;
    var num = function (x) { return Number(x); };
    var bools = [];
    if (o.bool !== undefined) {
      var targets = o.targets !== undefined ? convertParam('solids', o.targets, ctx) : [];
      if (!targets.length) throw new Error('bool needs targets');
      bools = [[NAMES.boolSetters.targets, targets], [NAMES.boolSetters.op, NAMES.boolOps[o.bool]]];
    }
    // 辅助曲线保留(同手工拉伸 / 旋转): 实测 2026-11-17, 在命令里 geo.removeItem 删掉它, 下一条命令报
    // "invalid precondition: object N missing from stable model"(稳定模型里还挂着它)。
    var finish = function (cmd, f) {
      bools.forEach(function (s) { f[s[0]] = s[1]; });
      return f[NAMES.facCommit]();
    };
    if (rc.custom === 'prism') {
      var n = o.normal !== undefined ? convertParam('axisQuat', o.normal, ctx) : SF.polygon.baseNormal.slice();
      var c = convertParam('vecset', o.center, ctx);
      var sides = Math.round(num(o.sides));
      if (!(sides >= 3 && sides <= 64)) throw new Error('sides must be 3..64');
      var R;
      if (o.radius !== undefined) R = convertParam('length', o.radius, ctx);
      else if (o.flats !== undefined) R = convertParam('length', o.flats, ctx) / (2 * Math.cos(Math.PI / sides));
      else throw new Error('prism needs radius (circumscribed) or flats (across flats)');
      if (!(R > 0)) throw new Error('radius / flats must be > 0');
      var h = convertParam('length', o.height, ctx);
      if (!h) throw new Error('height must not be 0');
      var u = perpOf(n), w = vcross(n, u), rot = o.rotate !== undefined ? convertParam('angle', o.rotate, ctx) : 0;
      var dir = [u[0] * Math.cos(rot) + w[0] * Math.sin(rot), u[1] * Math.cos(rot) + w[1] * Math.sin(rot), u[2] * Math.cos(rot) + w[2] * Math.sin(rot)];
      var Poly = need('fac.centerPolygon'), Ext = need(rc.fac), P = SF.polygon, S = SF.sweep;
      return function (cmd) {
        var before = allRegions().map(function (r) { return r.id; });
        var p = new Poly(cmd.editor);
        p[NAMES.facResource](cmd);
        p[P.center].set(c[0], c[1], c[2]);
        p[P.orientation].setFromUnitVectors(new V(P.baseNormal[0], P.baseNormal[1], P.baseNormal[2]), new V(n[0], n[1], n[2]));
        p[P.point].set(c[0] + dir[0] * R, c[1] + dir[1] * R, c[2] + dir[2] * R);
        p[P.sides] = sides;
        p[P.radius] = R;
        return Promise.resolve(p[NAMES.facCommit]()).then(function () {
          var region = pickNewRegion(before, c, k);
          var f = new Ext(cmd.editor);
          f[NAMES.facResource](cmd);
          f[S.regions] = [region];
          var sg = h < 0 ? -1 : 1;
          f[S.direction].set(n[0] * sg, n[1] * sg, n[2] * sg);
          f[S.distance] = Math.abs(h);
          return finish(cmd, f);
        });
      };
    }
    if (rc.custom === 'revolve') {
      var ax = convertParam('axisQuat', o.axis, ctx), org = convertParam('vecset', o.origin, ctx);
      var prof = o.profile;
      if (!Array.isArray(prof) || prof.length < 3) throw new Error('profile needs at least 3 [r, z] points');
      var uu = perpOf(ax);
      var pts = prof.map(function (q, i) {
        if (!Array.isArray(q) || q.length !== 2) throw new Error('profile point #' + i + ' must be [r, z]');
        var r = num(q[0]) * k, z = num(q[1]) * k;
        if (!isFinite(r) || !isFinite(z)) throw new Error('profile point #' + i + ' is not numeric');
        if (r < -1e-12) throw new Error('profile point #' + i + ': r must be >= 0 (the profile may not cross the axis)');
        return [org[0] + uu[0] * r + ax[0] * z, org[1] + uu[1] * r + ax[1] * z, org[2] + uu[2] * r + ax[2] * z];
      });
      var mn = [Infinity, Infinity, Infinity], mx = [-Infinity, -Infinity, -Infinity];
      pts.forEach(function (p) { for (var j = 0; j < 3; j++) { mn[j] = Math.min(mn[j], p[j]); mx[j] = Math.max(mx[j], p[j]); } });
      var mid = [(mn[0] + mx[0]) / 2, (mn[1] + mx[1]) / 2, (mn[2] + mx[2]) / 2];
      var deg = o.degrees !== undefined ? num(o.degrees) : 360;
      if (!(deg > 0 && deg <= 360)) throw new Error('degrees must be in (0, 360]');
      var Crv = need('fac.curve'), Rev = need(rc.fac), C = SF.curve, S2 = SF.sweep;
      return function (cmd) {
        var before = allRegions().map(function (r) { return r.id; });
        var cf = new Crv(cmd.editor);
        cf[NAMES.facResource](cmd);
        cf[C.type] = C.polyline;
        pts.forEach(function (p) { cf[C.push](new V(p[0], p[1], p[2])); });
        cf[C.closed] = true;
        return Promise.resolve(cf[NAMES.facCommit]()).then(function () {
          var region = pickNewRegion(before, mid, k);
          var f = new Rev(cmd.editor);
          f[NAMES.facResource](cmd);
          f[S2.regions] = [region];
          f[S2.origin].set(org[0], org[1], org[2]);
          f[S2.axis].set(ax[0], ax[1], ax[2]);
          f[S2.degrees] = deg;
          return finish(cmd, f);
        });
      };
    }
    var xform = xformBuild(o, rc, ctx);
    if (xform) return xform;
    var outl = outlineBuild(o, rc, ctx);
    if (outl) return outl;
    throw new Error('recipe: unknown custom builder ' + rc.custom);
  }

  // ---- 大纲: 文件夹 / 移动 / 隐藏 / 解散(节点键 = nodes.item2key; 文件夹 = 组对象) ----
  function outlineRoot() { return need('groups.root'); }
  function folderKey(g) { return invokeOn('nodes.item2key', [g]); }
  function allFolders() {
    var ids = Array.from(need('groups.all') || []);
    return ids.map(function (id) { return invokeOn('groups.byId', [id]); }).filter(function (g) { return g && g[NAMES.bom.groupId] !== NAMES.bom.rootGroupId; });
  }
  function folderName(g) { try { return invokeOn('nodes.getName', [folderKey(g)]); } catch (e) { return null; } }
  function resolveFolder(ref, ctx) {
    var O = NAMES.outline;
    if (ref === O.rootRef || ref === NAMES.bom.rootGroupId) return outlineRoot();
    if (typeof ref === 'string' && ref.charAt(0) === '$') {
      var f = ctx.folders && ctx.folders[ref.slice(1)];
      if (!f) throw new Error('unknown folder ref ' + ref);
      return f;
    }
    if (typeof ref === 'number') {
      var g = invokeOn('groups.byId', [ref]);
      if (!g || allFolders().indexOf(g) < 0) throw new Error('folder not found: ' + ref);
      return g;
    }
    var name = ref && typeof ref === 'object' ? ref.name : ref;
    if (typeof name !== 'string') throw new Error('folder ref must be an id, a name, a $ref or "root"');
    var m = allFolders().filter(function (x) { return folderName(x) === name; });
    if (!m.length) throw new Error('no folder named ' + JSON.stringify(name));
    if (m.length > 1) throw new Error('several folders named ' + JSON.stringify(name) + ' (ids ' + m.map(function (x) { return x[NAMES.bom.groupId]; }).join(', ') + '); use the id');
    return m[0];
  }
  function folderList(val, ctx) { return (Array.isArray(val) ? val : [val]).map(function (r) { return resolveFolder(r, ctx); }); }
  // 文件夹 a 在 b 里面(或就是 b)?
  function folderWithin(a, b) {
    var root = NAMES.bom.rootGroupId, k = folderKey(a);
    if (a === b) return true;
    for (var i = 0; i < 64; i++) {
      var pid = invokeOn('groups.parentId', [k]);
      if (pid === undefined || pid === null) return false;
      if (pid === b[NAMES.bom.groupId]) return true;
      if (pid === root) return false;
      k = folderKey(invokeOn('groups.byId', [pid]));
    }
    return false;
  }
  function outlineBuild(o, rc, ctx) {
    var O = NAMES.outline;
    var solids = o.solids === undefined ? [] : convertParam('solids', o.solids, ctx);
    var folders = o.folders === undefined ? [] : folderList(o.folders, ctx);
    if (folders.indexOf(outlineRoot()) >= 0) throw new Error('the top level is not a folder you can move / hide / dissolve');
    var keys = function () { return solids.map(function (v) { return invokeOn('nodes.item2key', [v]); }).concat(folders.map(folderKey)); };
    // 包一层: 做完重算场景可见性(隐藏 / 进出隐藏的文件夹都会改变可见集合)
    var andRefresh = function (fn) { return function () { var r = fn(); if (get('scene.update')) invokeOn('scene.update', []); return r; }; };
    var moveInto = function (target) {
      folders.forEach(function (f) { if (folderWithin(target, f)) throw new Error('cannot move folder ' + JSON.stringify(folderName(f)) + ' into itself'); });
      keys().forEach(function (k) { invokeOn('groups.move', [k, target]); });
    };
    switch (rc.custom) {
      case 'folder': {
        var nm = String(o.name).trim();
        if (!nm) throw new Error('name must not be empty');
        var parent = o.parent === undefined ? outlineRoot() : resolveFolder(o.parent, ctx);
        return andRefresh(function () {
          var g = invokeOn('groups.create', []);
          invokeOn('groups.attach', [folderKey(g), parent]);
          invokeOn('nodes.setName', [folderKey(g), nm]);
          moveInto(g);
          if (o.id) { ctx.folders = ctx.folders || {}; ctx.folders[o.id] = g; }
          return [g].concat(solids);
        });
      }
      case 'moveTo': {
        if (!solids.length && !folders.length) throw new Error('moveTo needs solids or folders');
        var to = resolveFolder(o.to, ctx);
        return andRefresh(function () { moveInto(to); return folders.concat(solids); });
      }
      case 'hide':
      case 'show': {
        if (!solids.length && !folders.length) throw new Error(rc.custom + ' needs solids or folders');
        var hid = rc.custom === 'hide';
        return andRefresh(function () { keys().forEach(function (k) { invokeOn('nodes.setHidden', [k, hid]); }); return folders.concat(solids); });
      }
      case 'ungroup':
        if (!folders.length) throw new Error('ungroup needs folders');
        return andRefresh(function () { folders.forEach(function (f) { invokeOn('groups.dissolve', [f]); }); return []; });
    }
    return null;
  }

  // 大纲树(只读): 文件夹嵌套 + 物体(kind / id / 名字 / 隐藏)。物体 id = 实体版本 id(与 model_* 其余工具一致), 其他类别给节点键。
  function outlineNode(k, depth) {
    var O = NAMES.outline, it;
    try { it = invokeOn('nodes.key2item', [k]); } catch (e) { return null; }
    var ctor = it && it.constructor && it.constructor.name;
    var hidden = false;
    try { hidden = !!invokeOn('nodes.isHidden', [k]) || !invokeOn('nodes.isVisible', [k]); } catch (e) {}
    if (ctor === O.groupCtor) {
      var kids = depth > 32 ? [] : Array.from(invokeOn('groups.children', [it]) || []).map(function (c) { return outlineNode(c, depth + 1); }).filter(Boolean);
      return { kind: 'folder', id: it[NAMES.bom.groupId], name: invokeOn('nodes.getName', [k]), hidden: hidden, children: kids };
    }
    var n = { kind: O.kinds[ctor] || ctor, name: null, hidden: hidden };
    try { n.name = invokeOn('nodes.getName', [k]); } catch (e) {}
    var vid = itemIdOf(it);
    if (n.kind === 'solid' && vid !== null) n.id = vid; else n.key = k;
    try { if (invokeOn('nodes.isLocked', [k])) n.locked = true; } catch (e) {}
    return n;
  }
  def('outline.read', 'outline.read', function (a) {
    var kinds = a && Array.isArray(a.kinds) ? a.kinds : null;
    var tree = Array.from(invokeOn('groups.children', [outlineRoot()]) || []).map(function (c) { return outlineNode(c, 0); }).filter(Boolean);
    var prune = function (list) {
      return list.map(function (n) { if (n.kind === 'folder') { n.children = prune(n.children); return n; } return !kinds || kinds.indexOf(n.kind) >= 0 ? n : null; }).filter(Boolean);
    };
    return { children: prune(tree) };
  });

  // ---- A4b: 整体操作(复制 / 镜像 / 移动 / 旋转 / 阵列); 参数在入队前全部换算好 ----
  function xformBuild(o, rc, ctx) {
    var X = NAMES.xformFields, V = ctx.V3;
    var count = function (x, name) {
      var n = Math.round(Number(x));
      if (!(n >= 1 && n <= 1000)) throw new Error(name + ' must be 1..1000');
      return n;
    };
    var make = function (sets) {
      var Fac = need(rc.fac);
      return function (cmd) {
        var f = new Fac(cmd.editor);
        f[NAMES.facResource](cmd);
        sets(f);
        return f[NAMES.facCommit]();
      };
    };
    if (rc.custom === 'boolean') {
      if (!(o.operation in NAMES.boolOps)) throw new Error('operation must be one of ' + Object.keys(NAMES.boolOps).join(', '));
      var tg = convertParam('solids', o.targets, ctx), tl = convertParam('solids', o.tools, ctx);
      if (!tg.length || !tl.length) throw new Error('boolean needs targets and tools');
      if (tl.some(function (v) { return tg.indexOf(v) >= 0; })) throw new Error('a solid cannot be both target and tool');
      return make(function (f) {
        f[NAMES.boolSetters.targets] = tg; f[X.tools] = tl; f[NAMES.boolSetters.op] = NAMES.boolOps[o.operation];
        if (o.keepTools !== undefined) f[X.keepTools] = !!o.keepTools;
      });
    }
    if (NAMES.outline.ops.indexOf(rc.custom) >= 0) return null;   // 大纲操作 → outlineBuild
    var items = rc.custom === 'mirror' ? X.mirrorItems : X.items;
    var solids = rc.custom === 'rename' && o.solids === undefined ? [] : convertParam('solids', o.solids, ctx);
    if (!solids.length && !(rc.custom === 'rename' && o.folders !== undefined)) throw new Error('solids is empty');
    var put = function (f) { f[items] = solids; };
    if (rc.custom === 'delete') {
      return function () {
        return Promise.all(solids.map(function (v) { return invokeOn('geo.removeItem', [v]); })).then(function () { return []; });
      };
    }
    if (rc.custom === 'rename') {
      var nm = String(o.name).trim();
      if (!nm) throw new Error('name must not be empty');
      var fs = o.folders === undefined ? [] : folderList(o.folders, ctx);
      if (fs.indexOf(outlineRoot()) >= 0) throw new Error('the top level cannot be renamed');
      return function () {
        solids.forEach(function (v) { invokeOn('nodes.setName', [invokeOn('nodes.item2key', [v]), nm]); });
        fs.forEach(function (g) { invokeOn('nodes.setName', [folderKey(g), nm]); });
        return fs.concat(solids);
      };
    }
    switch (rc.custom) {
      case 'duplicate':
        return make(put);
      case 'mirror': {
        var org = convertParam('vecset', o.origin, ctx), nrm = convertParam('axisQuat', o.normal, ctx);
        return make(function (f) { put(f); f[X.origin].set(org[0], org[1], org[2]); f[X.normal].set(nrm[0], nrm[1], nrm[2]); });
      }
      case 'move': {
        var mv = convertParam('translation', o.move, ctx);
        return make(function (f) { put(f); f[X.move].set(mv[0], mv[1], mv[2]); });
      }
      case 'rotate': {
        var pv = convertParam('vecset', o.pivot, ctx), ax = convertParam('axisQuat', o.axis, ctx), ang = convertParam('angle', o.degrees, ctx);
        if (!ang) throw new Error('degrees must not be 0');
        return make(function (f) { put(f); f[X.pivot].set(pv[0], pv[1], pv[2]); f[X.axis].copy(new V(ax[0], ax[1], ax[2])); f[X.angle] = ang; });
      }
      case 'pattern': {
        var d1 = convertParam('axisQuat', o.dir1, ctx), n1 = count(o.count1, 'count1'), s1 = convertParam('length', o.spacing1, ctx);
        var two = o.dir2 !== undefined || o.count2 !== undefined || o.spacing2 !== undefined;
        if (two && (o.dir2 === undefined || o.count2 === undefined || o.spacing2 === undefined)) throw new Error('second direction needs dir2, count2 and spacing2');
        var d2 = two ? convertParam('axisQuat', o.dir2, ctx) : perpOf(d1);
        var n2 = two ? count(o.count2, 'count2') : 1, s2 = two ? convertParam('length', o.spacing2, ctx) : 0;
        if (n1 * n2 < 2) throw new Error('pattern needs at least 2 instances');
        if (two && Math.abs(vdot(d1, d2)) > 1 - 1e-9) throw new Error('dir1 and dir2 must not be parallel');
        return make(function (f) {
          put(f);
          f[X.dir1] = new V(d1[0], d1[1], d1[2]); f[X.dir2] = new V(d2[0], d2[1], d2[2]);
          f[X.num1] = n1; f[X.num2] = n2;
          f[X.dist1] = s1 * (n1 - 1); f[X.dist2] = s2 * (n2 - 1);
        });
      }
      case 'radialPattern': {
        var c = convertParam('vecset', o.center, ctx), axr = convertParam('axisQuat', o.axis, ctx), n = count(o.count, 'count');
        if (n < 2) throw new Error('count must be >= 2');
        var deg = o.degrees !== undefined ? Number(o.degrees) : 360;
        if (!(deg > 0 && deg <= 360)) throw new Error('degrees must be in (0, 360]');
        var st = perpOf(axr);
        return make(function (f) {
          put(f);
          f[X.dir1] = new V(st[0], st[1], st[2]); f[X.dir2] = new V(axr[0], axr[1], axr[2]);
          f[X.center].set(c[0], c[1], c[2]);
          f[X.radialCount] = n; f[X.radialAngle] = deg * Math.PI / 180;
        });
      }
    }
    return null;
  }

  // 一步 → build(cmd) 函数。参数在入队前全部换算好(选择器出错就不产生命令)。
  function buildStep(o, ctx) {
    var rc = NAMES.recipes[o.op];
    if (rc.custom) return customBuild(o, rc, ctx);
    var Fac = need(rc.fac);
    var sets = [];
    Object.keys(rc.params).forEach(function (p) {
      if (o[p] === undefined) return;
      var type = rc.params[p][1], val = convertParam(type, o[p], ctx);
      var setter = rc.params[p][0];
      if (type === 'translation') {
        // 平移: 向量与平移矩阵都要设(见 recipes.moveFaces)
        var TF = NAMES.translationFields;
        sets.push(function (f) { f[TF.vector].set(val[0], val[1], val[2]); f[TF.matrix].makeTranslation(val[0], val[1], val[2]); });
      } else if (type === 'vecset') {
        sets.push(function (f) { f[setter].set(val[0], val[1], val[2]); });
      } else if (type === 'axisQuat') {
        sets.push(function (f) {
          var V = ctx.V3, b = NAMES.cylinderFields.baseAxis;
          f[setter].setFromUnitVectors(new V(b[0], b[1], b[2]), new V(val[0], val[1], val[2]));
        });
      } else {
        sets.push([setter, val]);
      }
    });
    if (rc.radiusPoint) {
      // N4 圆柱: 半径点 = 圆心 + 垂直于轴的方向 × 半径(手工建圆柱时的第二次点击); 设完再设一次半径
      var CF = NAMES.cylinderFields;
      var cen = convertParam('vecset', o.center, ctx), ax = convertParam('axisQuat', o.axis, ctx), rr = convertParam('length', o.radius, ctx);
      var pp = Math.abs(ax[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0];
      var dp = pp[0] * ax[0] + pp[1] * ax[1] + pp[2] * ax[2];
      pp = [pp[0] - dp * ax[0], pp[1] - dp * ax[1], pp[2] - dp * ax[2]];
      var pl = Math.hypot(pp[0], pp[1], pp[2]);
      sets.push(function (f) {
        f[CF.point].set(cen[0] + pp[0] / pl * rr, cen[1] + pp[1] / pl * rr, cen[2] + pp[2] / pl * rr);
        f[CF.radius] = rr;
      });
    }
    if (o.bool !== undefined) {
      var targets = o.targets !== undefined ? convertParam('solids', o.targets, ctx) : [];
      if (!targets.length) throw new Error('bool needs targets');
      sets.push([NAMES.boolSetters.targets, targets]);
      sets.push([NAMES.boolSetters.op, NAMES.boolOps[o.bool]]);
    }
    return function (cmd) {
      var f = new Fac(cmd.editor);
      f[NAMES.facResource](cmd);
      sets.forEach(function (s) { if (typeof s === 'function') s(f); else f[s[0]] = s[1]; });
      return f[NAMES.facCommit]();
    };
  }

  function checkStep(o, i) {
    var at = 'step #' + i + ' (' + (o && o.op) + '): ';
    var rc = o && NAMES.recipes[o.op];
    if (!rc) throw new Error(at + 'unknown op (known: ' + Object.keys(NAMES.recipes).join(', ') + ')');
    rc.required.forEach(function (p) { if (o[p] === undefined) throw new Error(at + 'missing ' + p); });
    Object.keys(o || {}).forEach(function (p) {
      if (p !== 'op' && p !== 'id' && p !== 'bool' && p !== 'targets' && !rc.params[p]) throw new Error(at + 'unknown param ' + p);
    });
    if (o.bool !== undefined) {
      if (!rc.bool) throw new Error(at + 'bool not supported');
      if (!(o.bool in NAMES.boolOps)) throw new Error(at + 'bool must be one of ' + Object.keys(NAMES.boolOps).join(', '));
    }
    need(rc.fac);
    (rc.also || []).forEach(function (f) { need(f); });
  }

  // =====================================================================
  // A3: 事务回滚。begin 记下撤销栈(深度 + 每一项的对象引用); rollback 逐步派发 edit:undo 撤到 begin 时的深度。
  // 安全: begin 位置以下的撤销栈必须原样(用户撤到 begin 之前 / 栈被换掉 → 拒绝, 不乱撤)。
  // 撤销走 memento 恢复 → 窗口不可见时拒绝(mementoGuard)。
  // =====================================================================
  function undoStack() { var s = need('ed.undoStack'); return s; }

  function markNow() { var s = undoStack(); return { depth: s.length, base: Array.from(s) }; }

  function checkMark(mark) {
    var s = undoStack();
    if (s.length < mark.depth) throw new Error('undo stack is below the transaction start (' + s.length + ' < ' + mark.depth + ')');
    for (var i = 0; i < mark.depth; i++) if (s[i] !== mark.base[i]) throw new Error('undo history changed below the transaction start (entry ' + i + ')');
  }

  function undoOnce() {
    var before = undoStack().length;
    W.document.body.dispatchEvent(new CustomEvent(NAMES.undoCommand, { bubbles: true }));
    return new Promise(function (resolve, reject) {
      var t0 = Date.now();
      (function poll() {
        if (undoStack().length < before) return resolve();
        if (Date.now() - t0 > NAMES.undoStepTimeoutMs) return reject(new Error('undo did not complete within ' + NAMES.undoStepTimeoutMs + ' ms'));
        setTimeout(poll, NAMES.undoPollMs);
      })();
    });
  }

  // 撤到 mark.depth; 返回撤销的步数。
  function rollbackTo(mark) {
    checkMark(mark);
    mementoGuard();
    var n = 0;
    var loop = function () {
      if (undoStack().length <= mark.depth) return Promise.resolve(n);
      return undoOnce().then(function () { n++; return loop(); });
    };
    return loop();
  }

  // 事务记在页面级 __PLA_PAGE__ 上(跨垫片实例存续): 实测 2026-11-17, begin 之后垫片热替换(版本变了)→ 记录丢了, rollback 报 no transaction。
  // 事务可嵌套(栈): begin 压栈, commit / rollback 弹最里层 —— 外层(例如还没收尾的测试事务)不受影响(2026-11-18)。
  function txStack() {
    var p = pageState();
    if (!p.txs) { p.txs = p.tx ? [p.tx] : []; p.tx = null; }
    return p.txs;
  }

  def('hist.begin', 'hist.write', function () {
    var s = txStack(), m = markNow();
    s.push(m);
    return { depth: m.depth, level: s.length };
  });

  def('hist.commit', 'hist.write', function () {
    var s = txStack(), m = s.pop();
    return { committed: !!m, steps: m ? undoStack().length - m.depth : 0, level: s.length };
  });

  // 回滚最里层事务(它 begin 之后的全部步骤, 含期间用户自己的操作); 结束这一层。
  def('hist.rollback', 'hist.write', function () {
    var s = txStack();
    if (!s.length) throw new Error('no transaction (call hist.begin first)');
    var mark = s[s.length - 1];
    return rollbackTo(mark).then(function (n) { s.pop(); return { undone: n, depth: undoStack().length, level: s.length }; });
  }, { async: true });

  // 撤销最近 steps 步(同 Ctrl+Z, 逐步确认)。
  def('hist.undo', 'hist.write', function (a) {
    var steps = Math.floor(Number((a || {}).steps || 1));
    var s = undoStack();
    if (!(steps >= 1) || steps > s.length) throw new Error('steps must be 1..' + s.length);
    var mark = { depth: s.length - steps, base: Array.from(s).slice(0, s.length - steps) };
    return rollbackTo(mark).then(function (n) { return { undone: n, depth: undoStack().length }; });
  }, { async: true });

  // 工厂挂载状态(只读): 供 Rust 判断要不要让代理拍快照、自检显示。
  def('cmd.factories', 'cmd.exec', function () {
    var F = W[CFG.factoriesGlobal];
    var keys = {};
    Object.keys(NAMES.recipes).forEach(function (op) { keys[NAMES.recipes[op].fac] = true; (NAMES.recipes[op].also || []).forEach(function (f) { keys[f] = true; }); });
    var missing = Object.keys(keys).filter(function (k) { return !resolve(k).ok; });
    return { loaded: !!F, count: F ? Object.keys(F).length : 0, missing: missing, ops: Object.keys(NAMES.recipes) };
  });

  // 执行一串步骤。args = { ops: [{op, id?, <参数>..., bool?, targets?}], units?: 文档单位名(默认取文档设置), atomic? }。
  // 每步一个命令(一个撤销步); 某步失败即停, 已完成的保留并如实返回:
  // { units, results: [{index, op, id, items:[{kind, id, name, bbox}]}], failed: null | {index, op, err} }。
  // atomic: 失败时撤回本次全部已完成步骤 → results 清空, 带 rolledBack(撤销步数); 撤回失败带 rollbackErr。
  def('cmd.exec', 'cmd.exec', function (a) {
    if (!a || !Array.isArray(a.ops) || !a.ops.length) throw new Error('cmd.exec needs {ops: [...]}');
    if (!W[CFG.factoriesGlobal]) throw new Error(CFG.needFactoriesErr);
    var U = unitOf(a);
    a.ops.forEach(checkStep);
    var ctx = { k: U.k, V3: need('vp.camera').position.constructor, refs: {} };
    var Cls = factoryCommandClass();
    // A3 atomic: 某步失败 → 撤回本次已完成的全部步骤(窗口不可见时撤不了 → 一开始就拒绝)
    var mark = null;
    if (a.atomic) {
      if (!resolve('ed.undoStack').ok) throw new CapError('ed.undoStack', ['ed.undoStack']);
      if (W.document && W.document.visibilityState === 'hidden') throw new Error('atomic needs a visible window (undo uses memento restore)');
      mark = markNow();
    }
    var results = [];
    var finish = function (failed) { return { units: U.unit, results: results, failed: failed }; };
    var done = function (failed) {
      if (!failed || !mark) return Promise.resolve(finish(failed));
      return rollbackTo(mark).then(function (n) {
        var r = finish(failed); r.rolledBack = n; r.results = []; return r;
      }, function (e) {
        var r = finish(failed); r.rollbackErr = errMsg(e); return r;
      });
    };
    var step = function (i) {
      if (i >= a.ops.length) return done(null);
      var o = a.ops[i];
      var build;
      try { build = buildStep(o, ctx); } catch (e) { return done({ index: i, op: o.op, err: errMsg(e) }); }
      var cmd = new Cls(editor(), build);
      return runCommand(cmd, NAMES.cmdTimeoutMs).then(function (how) {
        if (cmd.plaErr || how !== 'ended') return done({ index: i, op: o.op, err: cmd.plaErr || how });
        var made = cmd.plaResult === null || cmd.plaResult === undefined ? [] : (Array.isArray(cmd.plaResult) ? cmd.plaResult : [cmd.plaResult]);
        if (o.id) ctx.refs[o.id] = made.filter(function (v) { return itemIdOf(v) !== null; });
        results.push({ index: i, op: o.op, id: o.id || null, items: made.map(function (v) { return describeMade(v, U.k); }) });
        return step(i + 1);
      });
    };
    return step(0);
  }, { async: true });

  // =====================================================================
  // A1: 几何查询(只读; PRD-ai-modeling-v2 §2/§4 的几何签名)
  // 长度按文档单位(或 args.units)输出, 方向是单位向量; 数值保留 7 位有效数字(Float32 / 浮点噪声)。
  // =====================================================================
  function sig7(x) { var n = Number(Number(x).toPrecision(7)); return Math.abs(n) < 1e-12 ? 0 : n; }
  function vecOut(v, k) { return v && typeof v.x === 'number' ? [sig7(v.x / k), sig7(v.y / k), sig7(v.z / k)] : null; }
  function dirOut(v) { return vecOut(v, 1); }
  function box6Out(b, o, k) { return [[sig7(b[o] / k), sig7(b[o + 1] / k), sig7(b[o + 2] / k)], [sig7(b[o + 3] / k), sig7(b[o + 4] / k), sig7(b[o + 5] / k)]]; }
  function boxCenter(bb) { return [(bb[0][0] + bb[1][0]) / 2, (bb[0][1] + bb[1][1]) / 2, (bb[0][2] + bb[1][2]) / 2]; }

  // 组(面组/边组)里的项: [{id, bbox(文档单位)}], 与 versionIds 同序
  function groupEntries(view, kind, k) {
    var g = view && view[NAMES.shellGroups[kind]];
    var ids = g && g[NAMES.groupVersionIds], bx = g && g[NAMES.groupBboxes];
    if (!ids || !bx) return [];
    return Array.from(ids).map(function (id, i) { return { id: String(id), bbox: box6Out(bx, i * 6, k) }; });
  }

  function modelOf(kind, id) { var t = lookupOf(kind, id); return t ? t[NAMES.lookupModelKey] : null; }

  // 曲面/曲线 GetInfo 里的半径 + 轴(圆柱/圆锥/圆/圆弧…); 平面/直线没有半径
  function radialInfo(geom, o, k) {
    var A = NAMES.geoApi;
    var info = geom && typeof geom[A.info] === 'function' ? geom[A.info]() : null;
    if (!info) return;
    if (typeof info[A.infoRadius] === 'number') {
      o.radius = sig7(info[A.infoRadius] / k);
      var b = info[A.infoBasis];
      if (b) { o.axisOrigin = vecOut(b[A.basisLoc], k); o.axis = dirOut(b[A.basisAxis]); }
    } else if (info[A.lineDir]) {
      o.direction = dirOut(info[A.lineDir]);
    }
  }

  function faceSig(e, solidId, k) {
    var A = NAMES.geoApi;
    var o = { id: e.id, solid: solidId, bbox: e.bbox };
    var m = modelOf('face', e.id);
    if (!m) { o.err = 'lookup failed'; return o; }
    try {
      var surf = m[NAMES.surfaceGetter]()[A.surfaceKey];
      o.surface = surf && surf.constructor ? surf.constructor.name : null;
      radialInfo(surf, o, k);
    } catch (x) { o.err = errMsg(x); }
    try { var mp = m[A.midpoint](); o.center = vecOut(mp[A.midPos], k); o.normal = dirOut(mp[A.midNormal]); } catch (x2) {}
    return o;
  }

  function edgeSig(e, solidId, k, verts) {
    var A = NAMES.geoApi;
    var o = { id: e.id, solid: solidId, bbox: e.bbox };
    var m = modelOf('edge', e.id);
    if (!m) { o.err = 'lookup failed'; return o; }
    try {
      var c = m[A.curve]()[A.curveKey];
      o.curve = c && c.constructor ? c.constructor.name : null;
      radialInfo(c, o, k);
    } catch (x) { o.err = errMsg(x); }
    try { o.length = sig7(m[A.length]()[A.lengthKey] / k); } catch (x2) {}
    try {
      var lr = m[A.vertices]();
      [['start', A.vLeft], ['end', A.vRight]].forEach(function (p) {
        var v = lr[p[1]];
        if (!v) return;
        o[p[0]] = vecOut(v[A.point](), k);
        if (verts) {
          var vid = solidId + NAMES.vertexIdTag + v[A.ident]();
          if (!verts.has(vid)) verts.set(vid, { id: vid, solid: solidId, point: o[p[0]] });
        }
      });
    } catch (x3) {}
    return o;
  }

  // 过滤: surface / curve(类型名), normal + angleTol(度, 默认 1), near + tol(到包围盒中心, 文档单位, 默认 pickTolerance)
  function topoFilter(a) {
    var cosTol = Math.cos(((a.angleTol !== undefined ? Number(a.angleTol) : 1) * Math.PI) / 180);
    var n = a.normal;
    if (n !== undefined) {
      if (!Array.isArray(n) || n.length !== 3) throw new Error('normal must be [x,y,z]');
      var len = Math.hypot(n[0], n[1], n[2]);
      if (!(len > 0)) throw new Error('normal must be non-zero');
      n = n.map(function (x) { return x / len; });
    }
    if (a.near !== undefined && (!Array.isArray(a.near) || a.near.length !== 3)) throw new Error('near must be [x,y,z]');
    var tol = a.tol !== undefined ? Number(a.tol) : NAMES.pickTolerance;
    return {
      cheap: function (e) { // 只用包围盒(不调内核)
        if (!a.near) return true;
        var c = boxCenter(e.bbox);
        return Math.hypot(c[0] - a.near[0], c[1] - a.near[1], c[2] - a.near[2]) <= tol;
      },
      face: function (o) {
        if (a.surface && o.surface !== a.surface) return false;
        if (n && !(o.normal && o.normal[0] * n[0] + o.normal[1] * n[1] + o.normal[2] * n[2] >= cosTol)) return false;
        return true;
      },
      edge: function (o) { return !a.curve || o.curve === a.curve; }
    };
  }

  // args = { detail?: 'solids'(默认)|'faces'|'edges'|'all', of?: [物体 id | {name}], units?,
  //          surface?, curve?, normal?, angleTol?, near?, tol?, limit? }
  // 返回 { units, solids: [{id, name, bbox, faces, edges}], faces?: [...], edges?: [...], vertices?: [...], truncated? }
  def('geo.query', 'geo.read', function (a) {
    a = a || {};
    var detail = a.detail || 'solids';
    var want = { solids: ['solids'], faces: ['faces'], edges: ['edges'], all: ['faces', 'edges', 'vertices'] }[detail];
    if (!want) throw new Error('detail must be solids | faces | edges | all');
    var U = unitOf(a), k = U.k;
    var pool = a.of !== undefined ? (Array.isArray(a.of) ? a.of : [a.of]).reduce(function (acc, r) { return acc.concat(resolveSolids(r, {})); }, []) : solidViews();
    var flt = topoFilter(a);
    var limit = a.limit !== undefined ? Math.max(0, Number(a.limit)) : NAMES.queryLimit;
    var out = { units: U.unit, solids: [] };
    var total = {};
    var verts = want.indexOf('vertices') >= 0 ? new Map() : null;
    want.forEach(function (w) { if (w !== 'solids') { out[w] = []; total[w] = 0; } });
    pool.forEach(function (sv) {
      var sid = itemIdOf(sv);
      var fe = groupEntries(sv, 'face', k), ee = groupEntries(sv, 'edge', k);
      out.solids.push({ id: sid, name: nameOf(sv), bbox: bboxOf(sv, k), faces: fe.length, edges: ee.length });
      // 顶点优先取吸附数据(真顶点实体 id); 没有吸附数据才从边端点推(Parasolid 标识)
      var edgeVerts = verts;
      if (verts) {
        var sp = snapPointsOf(sv, ['vertex'], k);
        if (sp && sp.length) {
          edgeVerts = null;
          sp.forEach(function (p) { if (!verts.has(p.id)) verts.set(p.id, { id: p.id, solid: sid, point: p.pos }); });
        }
      }
      if (out.faces) fe.forEach(function (e) {
        if (!flt.cheap(e)) return;
        var o = faceSig(e, sid, k);
        if (!flt.face(o)) return;
        total.faces++;
        if (out.faces.length < limit) out.faces.push(o);
      });
      if (out.edges) ee.forEach(function (e) {
        if (!flt.cheap(e)) return;
        var o = edgeSig(e, sid, k, edgeVerts);
        if (!flt.edge(o)) return;
        total.edges++;
        if (out.edges.length < limit) out.edges.push(o);
      });
    });
    if (verts) { var vs = Array.from(verts.values()); total.vertices = vs.length; out.vertices = vs.slice(0, limit); }
    var tr = {};
    Object.keys(total).forEach(function (w) { if (total[w] > out[w].length) tr[w] = total[w] - out[w].length; });
    if (Object.keys(tr).length) out.truncated = tr;
    return out;
  });

  // =====================================================================
  // A2: 吸附点 + 测量(只读)。吸附点 = 测量 / 自由移动命令显示的那批点(面心、边中点、圆心、象限点、顶点、包围盒角),
  // 直接读 SnapScene 按物体缓存的数据, 不进命令、不碰界面(X-ray 下可点到的内部点也在里面)。
  // =====================================================================
  function snapEntryOf(view) {
    var C = get('snap.canonicals');
    if (!C || typeof C.forEach !== 'function') return null;
    var pre = NAMES.snapKeyPrefix + itemIdOf(view) + '|', hit = null;
    C.forEach(function (e, key) { if (!hit && String(key).indexOf(pre) === 0) hit = e; });
    return hit;
  }

  // 该物体缓存条目的实例变换(普通物体 = 单位阵 / 无); 实例化的物体可能有多个
  function occTransformsOf(entry) {
    var O = get('snap.occurrences'), out = [];
    if (O) Array.from(O).forEach(function (o) { if (o && o[NAMES.occEntry] === entry) out.push(o[NAMES.occTransform] || null); });
    return out.length ? out : [null];
  }

  function xfPoint(m, x, y, z) {
    if (!m) return [x, y, z];
    return [m[0] * x + m[4] * y + m[8] * z + m[12], m[1] * x + m[5] * y + m[9] * z + m[13], m[2] * x + m[6] * y + m[10] * z + m[14]];
  }
  function xfDir(m, x, y, z) {
    var v = m ? [m[0] * x + m[4] * y + m[8] * z, m[1] * x + m[5] * y + m[9] * z, m[2] * x + m[6] * y + m[10] * z] : [x, y, z];
    var l = Math.hypot(v[0], v[1], v[2]) || 1;
    return [sig7(v[0] / l), sig7(v[1] / l), sig7(v[2] / l)];
  }

  // 物体的吸附点: [{kind, solid, pos(文档单位), id?(拓扑 id), normal|tangent|axis?}]; 没有吸附数据 → null
  function snapPointsOf(view, kinds, k) {
    var entry = snapEntryOf(view);
    var sfi = entry && entry[NAMES.snapItemKey];
    if (!sfi) return null;
    var sid = itemIdOf(view), out = [];
    occTransformsOf(entry).forEach(function (m) {
      kinds.forEach(function (kind) {
        var spec = NAMES.snapKinds[kind];
        var blk = walk(sfi, spec[0]);
        if (blk === MISS || !blk) return;
        var P = blk[spec[3]];
        if (!P || !P.length) return;
        var ids = spec[1] ? blk[spec[1]] : null, D = spec[4] ? blk[spec[4]] : null;
        var n = Math.floor(P.length / 3), per = ids && ids.length ? n / ids.length : 1;
        for (var i = 0; i < n; i++) {
          var p = xfPoint(m, P[i * 3], P[i * 3 + 1], P[i * 3 + 2]);
          var o = { kind: kind, solid: sid, pos: [sig7(p[0] / k), sig7(p[1] / k), sig7(p[2] / k)] };
          if (ids) o.id = sid + NAMES.topoTag[spec[2]] + ids[Math.floor(i / per)];
          if (D && D.length) {
            var j = D.length === P.length ? i : Math.floor(i / per);
            if (j * 3 + 2 < D.length) o[spec[5]] = xfDir(m, D[j * 3], D[j * 3 + 1], D[j * 3 + 2]);
          }
          out.push(o);
        }
      });
    });
    return out;
  }

  // args = { of?, kinds?: [faceCenter|edgeMid|circleCenter|circleQuarter|vertex|bboxCorner], near?, tol?, units?, limit? }
  // → { units, points: [...], noSnapData: [物体 id](缓存里没有的物体), truncated? }
  def('geo.snaps', 'snap.read', function (a) {
    a = a || {};
    var U = unitOf(a), k = U.k;
    var kinds = a.kinds !== undefined ? (Array.isArray(a.kinds) ? a.kinds : [a.kinds]) : Object.keys(NAMES.snapKinds);
    kinds.forEach(function (x) { if (!NAMES.snapKinds[x]) throw new Error('unknown snap kind ' + x + ' (known: ' + Object.keys(NAMES.snapKinds).join(', ') + ')'); });
    if (a.near !== undefined && (!Array.isArray(a.near) || a.near.length !== 3)) throw new Error('near must be [x,y,z]');
    var tol = a.tol !== undefined ? Number(a.tol) : NAMES.pickTolerance;
    var limit = a.limit !== undefined ? Math.max(0, Number(a.limit)) : NAMES.queryLimit;
    var pool = a.of !== undefined ? (Array.isArray(a.of) ? a.of : [a.of]).reduce(function (acc, r) { return acc.concat(resolveSolids(r, {})); }, []) : solidViews();
    var out = { units: U.unit, points: [], noSnapData: [] }, total = 0;
    pool.forEach(function (sv) {
      var pts = snapPointsOf(sv, kinds, k);
      if (!pts) { out.noSnapData.push(itemIdOf(sv)); return; }
      pts.forEach(function (p) {
        if (a.near && Math.hypot(p.pos[0] - a.near[0], p.pos[1] - a.near[1], p.pos[2] - a.near[2]) > tol) return;
        total++;
        if (out.points.length < limit) out.points.push(p);
      });
    });
    if (total > out.points.length) out.truncated = total - out.points.length;
    return out;
  });

  // 测量参照 → { pos(米), dir?(单位向量), kind, planar? }。
  // 参照: [x,y,z](文档单位; 角度测量里当方向) / 拓扑 id 字符串('<物体>f<n>' 面 → 面心 + 法向; 'e' 边 → 中点 + 方向; 'v' 顶点)
  function measureRef(ref, k, asDir) {
    var A = NAMES.geoApi;
    if (Array.isArray(ref)) {
      if (ref.length !== 3) throw new Error('point must be [x,y,z]');
      if (asDir) { var l = Math.hypot(ref[0], ref[1], ref[2]); if (!(l > 0)) throw new Error('direction must be non-zero'); return { dir: ref.map(function (x) { return x / l; }), kind: 'vector' }; }
      return { pos: ref.map(function (x) { return Number(x) * k; }), kind: 'point' };
    }
    var m = /^(\d+)([a-z])(\d+)$/.exec(String(ref));
    if (!m) throw new Error('reference must be [x,y,z] or a topology id like 34f10088 / 34e10505 / 34v10495: ' + JSON.stringify(ref));
    var tag = m[2], sv = viewOf('solid', m[1]);
    if (!sv) throw new Error('solid not found: ' + m[1]);
    var kind = tag === NAMES.topoTag.face ? 'face' : tag === NAMES.topoTag.edge ? 'edge' : tag === NAMES.topoTag.vertex ? 'vertex' : null;
    if (!kind) throw new Error('unknown topology tag ' + tag);
    var snapKind = { face: 'faceCenter', edge: 'edgeMid', vertex: 'vertex' }[kind];
    var sp = (snapPointsOf(sv, [snapKind], 1) || []).filter(function (p) { return p.id === String(ref); })[0];
    var r = { kind: kind };
    if (sp) { r.pos = sp.pos; r.dir = sp.normal || sp.tangent || null; }
    if (kind === 'vertex') { if (!sp) throw new Error('vertex not found (no snap data): ' + ref); return r; }
    var model = modelOf(kind, String(ref));
    if (!model) throw new Error(kind + ' not found: ' + ref);
    if (kind === 'face') {
      var surf = model[NAMES.surfaceGetter]()[A.surfaceKey];
      r.planar = !!(surf && surf.constructor && surf.constructor.name === NAMES.planeSurface);
      if (!sp) { var mp = model[A.midpoint](); var mpos = vecOut(mp[A.midPos], 1); r.pos = mpos; r.dir = dirOut(mp[A.midNormal]); }
    } else {
      var info = {};
      try { radialInfo(model[A.curve]()[A.curveKey], info, 1); } catch (e) {}
      if (info.direction) r.dir = info.direction; // 直线: 曲线方向优于吸附切向
      if (!r.pos) { var lr = model[A.vertices](); var p0 = vecOut(lr[A.vLeft][A.point](), 1), p1 = vecOut(lr[A.vRight][A.point](), 1); r.pos = [(p0[0] + p1[0]) / 2, (p0[1] + p1[1]) / 2, (p0[2] + p1[2]) / 2]; }
    }
    return r;
  }

  // 物体的面积 / 体积 / 质心(显示网格; 曲面近似)。faces=true 时附逐面面积。
  function massOf(view, k, withFaces) {
    var fg = view[NAMES.shellGroups.face];
    var mesh = fg && fg[NAMES.faceMesh];
    var g = mesh && mesh.geometry;
    var pos = g && g.attributes && g.attributes.position && g.attributes.position.array;
    var idx = g && g.index && g.index.array;
    if (!pos || !idx) throw new Error('no display mesh for solid ' + itemIdOf(view));
    var me = mesh.matrixWorld && mesh.matrixWorld.elements;
    var P = function (i) { var j = idx[i] * 3; return xfPoint(me, pos[j], pos[j + 1], pos[j + 2]); };
    var grp = fg[NAMES.meshGroups] ? Array.from(fg[NAMES.meshGroups]) : [0, idx.length];
    var ids = fg[NAMES.groupVersionIds] ? Array.from(fg[NAMES.groupVersionIds]) : [];
    var area = 0, vol = 0, cx = 0, cy = 0, cz = 0, faces = [];
    for (var f = 0; f * 2 + 1 < grp.length; f++) {
      var fa = 0;
      for (var i = grp[f * 2]; i + 2 < grp[f * 2] + grp[f * 2 + 1]; i += 3) {
        var a = P(i), b = P(i + 1), c = P(i + 2);
        var ux = b[0] - a[0], uy = b[1] - a[1], uz = b[2] - a[2], vx = c[0] - a[0], vy = c[1] - a[1], vz = c[2] - a[2];
        fa += Math.hypot(uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx) / 2;
        var v6 = (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6;
        vol += v6; cx += v6 * (a[0] + b[0] + c[0]) / 4; cy += v6 * (a[1] + b[1] + c[1]) / 4; cz += v6 * (a[2] + b[2] + c[2]) / 4;
      }
      area += fa;
      if (withFaces) faces.push({ id: ids[f] !== undefined ? String(ids[f]) : null, area: sig7(fa / (k * k)) });
    }
    // 网格坐标是 Float32: 离原点 ~200 mm 处量 5 mm 宽的小面, 噪声已到第 6 位(实测 100 → 99.9999)
    // → 网格量只保留 5 位有效数字(本来就是近似值)
    var s6 = function (x) { var n = Number(Number(x).toPrecision(5)); return Math.abs(n) < 1e-12 ? 0 : n; };
    var r = { id: itemIdOf(view), name: nameOf(view), volume: s6(Math.abs(vol) / (k * k * k)), area: s6(area / (k * k)),
              centroid: vol ? [s6(cx / vol / k), s6(cy / vol / k), s6(cz / vol / k)] : null, method: 'mesh' };
    if (withFaces) r.faces = faces.map(function (x) { return { id: x.id, area: s6(x.area) }; });
    return r;
  }

  function angleDeg(u, v) { var d = u[0] * v[0] + u[1] * v[1] + u[2] * v[2]; return Math.acos(Math.max(-1, Math.min(1, d))) * 180 / Math.PI; }

  // args = { op: 'distance', a, b } | { op: 'angle', a, b } | { op: 'mass', of?, faces? };  units? 可覆盖。
  // distance → { distance, delta, a, b, normalDistance?(两平面平行时: 沿法向的距离 = 壁厚) }
  // angle    → { angle(度, 0–180) }(面取法向, 边取方向, 数组当方向向量)
  // mass     → { solids: [{id, name, volume, area, centroid, faces?, method: 'mesh'}] }
  def('geo.measure', 'geo.read', function (a) {
    a = a || {};
    var U = unitOf(a), k = U.k;
    var out = { units: U.unit, op: a.op };
    if (a.op === 'distance') {
      if (a.a === undefined || a.b === undefined) throw new Error('distance needs a and b');
      var A1 = measureRef(a.a, k), B1 = measureRef(a.b, k);
      var d = [B1.pos[0] - A1.pos[0], B1.pos[1] - A1.pos[1], B1.pos[2] - A1.pos[2]];
      out.distance = sig7(Math.hypot(d[0], d[1], d[2]) / k);
      out.delta = d.map(function (x) { return sig7(x / k); });
      out.a = A1.pos.map(function (x) { return sig7(x / k); }); out.b = B1.pos.map(function (x) { return sig7(x / k); });
      if (A1.planar && B1.planar && A1.dir && B1.dir) {
        var ang = angleDeg(A1.dir, B1.dir);
        if (ang <= NAMES.parallelTolDeg || ang >= 180 - NAMES.parallelTolDeg) out.normalDistance = sig7(Math.abs(d[0] * A1.dir[0] + d[1] * A1.dir[1] + d[2] * A1.dir[2]) / k);
      }
      return out;
    }
    if (a.op === 'angle') {
      if (a.a === undefined || a.b === undefined) throw new Error('angle needs a and b');
      var A2 = measureRef(a.a, k, true), B2 = measureRef(a.b, k, true);
      if (!A2.dir || !B2.dir) throw new Error('angle needs directions (faces, straight edges or vectors)');
      out.angle = sig7(angleDeg(A2.dir, B2.dir));
      return out;
    }
    if (a.op === 'mass') {
      var pool = a.of !== undefined ? (Array.isArray(a.of) ? a.of : [a.of]).reduce(function (acc, r) { return acc.concat(resolveSolids(r, {})); }, []) : solidViews();
      out.solids = pool.map(function (sv) { try { return massOf(sv, k, !!a.faces); } catch (e) { return { id: itemIdOf(sv), err: errMsg(e) }; } });
      return out;
    }
    throw new Error('op must be distance | angle | mass');
  });

  // ---- 视角 / 视口杂项 ----
  def('cam.navigateToSelection', 'cam.nav', function () { invokeOn('vp.navigate', [], viewport()); return true; });
  def('cam.render', 'cam.read', function () { invokeOn('vp.render', [], viewport()); return true; });

  // R33d: 视口缩略图(PNG base64)。canvas 没开 preserveDrawingBuffer, 必须在渲染的同一个 JS 任务里读;
  // canvas 是透明的, 背景色来自页面 → 先铺最近的不透明祖先背景色。只含 3D 视口, 不含 Plasticity 的面板。
  def('cam.thumbnail', 'cam.read', function (a) {
    var V = viewport();
    var maxW = (a && a.maxWidth) || 960;
    put('vp.needsRender', true, V);
    invokeOn('vp.handleRender', [W.performance ? W.performance.now() : Date.now()], V);
    var src = need('vp.canvas', V);
    var scale = Math.min(1, maxW / src.width);
    var c = W.document.createElement('canvas');
    c.width = Math.max(1, Math.round(src.width * scale));
    c.height = Math.max(1, Math.round(src.height * scale));
    var g = c.getContext('2d');
    var bg = null;
    for (var el = src; el && !bg; el = el.parentElement) {
      var col = W.getComputedStyle ? W.getComputedStyle(el).backgroundColor : '';
      if (col && col !== 'transparent' && !/rgba\([^)]*,\s*0\)$/.test(col)) bg = col;
    }
    g.fillStyle = bg || '#c9c9c4';
    g.fillRect(0, 0, c.width, c.height);
    g.drawImage(src, 0, 0, c.width, c.height);
    var url = c.toDataURL('image/png');
    return { data: url.slice(url.indexOf(',') + 1), width: c.width, height: c.height, bg: bg };
  });
  // =====================================================================
  // A5: 指定视角截图。临时切正交 → 相机朝向按视角(朝向 + 上方向构造四元数) → 对准物体包围盒、按投影尺寸算缩放 →
  // 截图(cam.thumbnail) → 还原相机与投影模式(keep=true 时不还原)。只动视角, 不改文档。
  // =====================================================================
  function vnorm(v) { var l = Math.hypot(v[0], v[1], v[2]) || 1; return [v[0] / l, v[1] / l, v[2] / l]; }
  function vcross(a, b) { return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]; }
  function vdot(a, b) { return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]; }
  // 相机基: z = 朝向相机(目标 → 相机), x = 右, y = 上; 旋转矩阵 [x y z] → 四元数
  function viewBasis(dir, up) {
    var z = vnorm(dir), x = vnorm(vcross(up, z)), y = vcross(z, x);
    var m00 = x[0], m01 = y[0], m02 = z[0], m10 = x[1], m11 = y[1], m12 = z[1], m20 = x[2], m21 = y[2], m22 = z[2];
    var tr = m00 + m11 + m22, q;
    if (tr > 0) { var s = 0.5 / Math.sqrt(tr + 1); q = [(m21 - m12) * s, (m02 - m20) * s, (m10 - m01) * s, 0.25 / s]; }
    else if (m00 > m11 && m00 > m22) { var s1 = 2 * Math.sqrt(1 + m00 - m11 - m22); q = [0.25 * s1, (m01 + m10) / s1, (m02 + m20) / s1, (m21 - m12) / s1]; }
    else if (m11 > m22) { var s2 = 2 * Math.sqrt(1 + m11 - m00 - m22); q = [(m01 + m10) / s2, 0.25 * s2, (m12 + m21) / s2, (m02 - m20) / s2]; }
    else { var s3 = 2 * Math.sqrt(1 + m22 - m00 - m11); q = [(m02 + m20) / s3, (m12 + m21) / s3, 0.25 * s3, (m10 - m01) / s3]; }
    return { x: x, y: y, z: z, q: q };
  }

  // args = { view?: front|back|left|right|top|bottom|iso(默认), of?: [物体], margin?: 1.15, maxWidth?: 960, keep?: false }
  // → { view, data(PNG base64), width, height, target(文档单位), zoom, restored }
  def('cam.view', 'cam.write', function (a) {
    a = a || {};
    var name = a.view || 'iso';
    var spec = NAMES.camViews[name];
    if (!spec) throw new Error('view must be one of ' + Object.keys(NAMES.camViews).join(', '));
    var k = unitOf(a).k;
    var pool = a.of !== undefined ? (Array.isArray(a.of) ? a.of : [a.of]).reduce(function (acc, r) { return acc.concat(resolveSolids(r, {})); }, []) : solidViews();
    // 包围盒(米)
    var mn = [Infinity, Infinity, Infinity], mx = [-Infinity, -Infinity, -Infinity];
    pool.forEach(function (sv) {
      var b = bboxOf(sv, 1);
      if (!b) return;
      for (var j = 0; j < 3; j++) { mn[j] = Math.min(mn[j], b[0][j]); mx[j] = Math.max(mx[j], b[1][j]); }
    });
    if (!(mn[0] <= mx[0])) throw new Error('nothing to frame (no solids / no bounding box)');
    var V = viewport();
    var cam = need('vp.camera', V);
    var c0 = API['cam.get'].fn();
    var mode0 = (invokeOn('vp.snapshot', [], V).camera || {}).mode;
    var toggled = false;
    if (mode0 !== NAMES.orthoMode) { invokeOn('vp.togglePerspective', [], V); toggled = true; }
    var shot, zoom, center = [(mn[0] + mx[0]) / 2, (mn[1] + mx[1]) / 2, (mn[2] + mx[2]) / 2];
    try {
      var B = viewBasis(spec.dir, spec.up);
      // 包围盒 8 个角投影到屏幕轴上的半宽 / 半高
      var hw = 0, hh = 0, depth = 0;
      for (var i = 0; i < 8; i++) {
        var p = [(i & 1 ? mx : mn)[0] - center[0], (i & 2 ? mx : mn)[1] - center[1], (i & 4 ? mx : mn)[2] - center[2]];
        hw = Math.max(hw, Math.abs(vdot(p, B.x))); hh = Math.max(hh, Math.abs(vdot(p, B.y))); depth = Math.max(depth, Math.abs(vdot(p, B.z)));
      }
      var margin = a.margin !== undefined ? Number(a.margin) : NAMES.viewMargin;
      var top = cam[NAMES.camOrthoTop], aspect = cam[NAMES.camAspect] || 1;
      zoom = top / (Math.max(hh, hw / aspect, 1e-9) * margin);
      var R = Math.max((c0.orbit && c0.orbit.radius) || 1, depth * 4);
      var c = JSON.parse(JSON.stringify(c0));
      c.px = center[0] + B.z[0] * R; c.py = center[1] + B.z[1] * R; c.pz = center[2] + B.z[2] * R;
      c.qx = B.q[0]; c.qy = B.q[1]; c.qz = B.q[2]; c.qw = B.q[3];
      c.orbit = c.orbit || {};
      c.orbit.target = center; c.orbit.targetEnd = center;
      // 平移视图留下的焦点偏移要清零, 否则放大后画面被推离物体(实测: 只剩方向立方体)
      c.orbit.focal = [0, 0, 0]; c.orbit.focalEnd = [0, 0, 0];
      c.orbit.radius = R; c.orbit.radiusEnd = R;
      c.orbit.zoom = zoom; c.orbit.zoomEnd = zoom;
      API['cam.set'].fn(c);
      // shot: false = 只摆相机、不截(调用方隔一个任务再 cam.thumbnail, 然后 cam.viewRestore)。
      // 实测 2026-11-17(窗口可见时): 同一个同步调用里 set 完立刻渲染, 画面还是旧相机; 隔开一次调用再截就对了。
      if (a.shot === false) {
        return { view: name, target: center.map(function (x) { return sig7(x / k); }), zoom: sig7(zoom), prev: c0, toggled: toggled };
      }
      shot = API['cam.thumbnail'].fn({ maxWidth: a.maxWidth || 960 });
    } finally {
      if (!a.keep && a.shot !== false) {
        API['cam.set'].fn(c0);
        if (toggled) invokeOn('vp.togglePerspective', [], V);
      }
    }
    return { view: name, data: shot.data, width: shot.width, height: shot.height,
             target: center.map(function (x) { return sig7(x / k); }), zoom: sig7(zoom), restored: !a.keep };
  });
  // cam.view {shot: false} 之后还原: 相机 + 投影模式(切过正交才切回)。
  def('cam.viewRestore', 'cam.write', function (a) {
    a = a || {};
    if (a.prev) API['cam.set'].fn(a.prev);
    if (a.toggled) invokeOn('vp.togglePerspective', [], viewport());
    return { restored: true };
  });

  // =====================================================================
  // N3: 面的可改尺寸。平面 → 厚度(同一实体里最近的背对平行平面, 投影有重叠的优先);
  // 倒圆面 → 圆角半径(op = refillet, 保持相切); 普通圆柱等带半径的面 → 半径(op = offsetFace, 轴线不动;
  // 凸 = 法向背离轴线: 外偏变大; 凹(孔) = 法向指向轴线: 外偏变小)。
  // 改值 = cmd.exec { op, faces: [选中面], distance: (新 − 旧) × offsetSign }。
  // (倒圆面若用 offsetFace, 半径变了但圆角鼓出两侧平面 —— 实测, 所以要分开。)
  // =====================================================================
  // args = { face?: 拓扑 id(默认 = 当前选择里唯一的面), units?, angleTol?: 度(默认 0.1) }
  // → { face, solid, surface, kind: 'thickness'|'radius'|'filletRadius'|'none', value?, op?, offsetSign?, opposite?, normal?, axis?, units }
  def('dim.face', 'geo.read', function (a) {
    a = a || {};
    var id = a.face;
    if (!id) {
      var ids = idList(need('sel.selected'), 'face');
      if (ids.length !== 1) throw new Error('select exactly one face (selected faces: ' + ids.length + ')');
      id = ids[0];
    }
    id = String(id);
    var m = /^(\d+)[a-z]/.exec(id);
    var sv = m && viewOf('solid', m[1]);
    if (!sv) throw new Error('face not found: ' + id);
    var U = unitOf(a), k = U.k, sid = itemIdOf(sv);
    var entries = groupEntries(sv, 'face', k);
    var me = entries.filter(function (e) { return e.id === id; })[0];
    if (!me) throw new Error('face not found in its solid: ' + id);
    var f = faceSig(me, sid, k);
    var out = { face: id, solid: sid, surface: f.surface || null, units: U.unit, kind: 'none' };
    var blend = -1;
    try { var fm = modelOf('face', id); if (fm && typeof fm[NAMES.geoApi.blendRadius] === 'function') blend = fm[NAMES.geoApi.blendRadius](); } catch (e) {}
    if (typeof blend === 'number' && blend > 0) {
      out.kind = 'filletRadius'; out.value = sig7(blend / k); out.op = 'refillet'; out.offsetSign = 1;
      return out;
    }
    if (typeof f.radius === 'number' && f.axisOrigin && f.axis && f.center && f.normal) {
      var w = [f.center[0] - f.axisOrigin[0], f.center[1] - f.axisOrigin[1], f.center[2] - f.axisOrigin[2]];
      var along = vdot(w, f.axis);
      w = [w[0] - along * f.axis[0], w[1] - along * f.axis[1], w[2] - along * f.axis[2]];
      var convex = vdot(w, f.normal) > 0;
      out.kind = 'radius'; out.value = f.radius; out.axis = f.axis; out.convex = convex; out.op = 'offsetFace'; out.offsetSign = convex ? 1 : -1;
      return out;
    }
    if (f.surface !== NAMES.planeSurface || !f.normal || !f.center) return out;
    out.center = f.center;
    out.axes = planeAxes(f.normal);   // N4 打孔的偏移轴(界面标签与 hole.make 共用这一条规则)
    var n = f.normal, cosTol = Math.cos(((a.angleTol !== undefined ? Number(a.angleTol) : NAMES.parallelTolDeg) * Math.PI) / 180);
    // 屏幕外两条世界轴上的投影重叠(法向的主轴之外)
    var ax = [0, 1, 2].sort(function (p, q) { return Math.abs(n[q]) - Math.abs(n[p]); });
    var overlap = function (b1, b2) {
      return [ax[1], ax[2]].every(function (j) { return Math.min(b1[1][j], b2[1][j]) - Math.max(b1[0][j], b2[0][j]) > -1e-9; });
    };
    // 全部候选: 背对的平行平面, 在选中面"后方"(d > 0); 排序 = 投影重叠的在前, 再按距离近 → 远。
    // 也收轴线与选中面平行(轴 ⟂ n)的圆柱面(用户 2026-11-15): 孔(凹) = 到孔壁最近处(轴深 − r), 凸圆柱 = 到最远处(轴深 + r)
    // —— 都是该处外法向背对选中面的那一点; 这一点要落在面的范围内(沿 n 方向看包围盒), 半个圆柱面不在那一侧就不算。
    var sinTol = Math.sin(((a.angleTol !== undefined ? Number(a.angleTol) : NAMES.parallelTolDeg) * Math.PI) / 180);
    var alongN = function (bb) {
      var lo = Infinity, hi = -Infinity;
      for (var i = 0; i < 8; i++) {
        var c = vdot([(i & 1 ? bb[1] : bb[0])[0], (i & 2 ? bb[1] : bb[0])[1], (i & 4 ? bb[1] : bb[0])[2]], n);
        lo = Math.min(lo, c); hi = Math.max(hi, c);
      }
      return [lo, hi];
    };
    var cands = [];
    entries.forEach(function (e) {
      if (e.id === id) return;
      var o = faceSig(e, sid, k);
      if (!o.center) return;
      var d;
      if (o.surface === NAMES.planeSurface && o.normal) {
        if (vdot(o.normal, n) > -cosTol) return;
        d = vdot([f.center[0] - o.center[0], f.center[1] - o.center[1], f.center[2] - o.center[2]], n);
      } else if (typeof o.radius === 'number' && o.axis && o.axisOrigin && o.normal && Math.abs(vdot(o.axis, n)) <= sinTol) {
        var w = [o.center[0] - o.axisOrigin[0], o.center[1] - o.axisOrigin[1], o.center[2] - o.axisOrigin[2]];
        var al = vdot(w, o.axis);
        w = [w[0] - al * o.axis[0], w[1] - al * o.axis[1], w[2] - al * o.axis[2]];
        var cvx = vdot(w, o.normal) > 0;
        var depth = vdot([f.center[0] - o.axisOrigin[0], f.center[1] - o.axisOrigin[1], f.center[2] - o.axisOrigin[2]], n);
        d = cvx ? depth + o.radius : depth - o.radius;
        // 三个测量点都给(界面可切: 最近 / 轴心 / 最远); point = 自动选的那个
        var cylVals = { near: sig7(depth - o.radius), axis: sig7(depth), far: sig7(depth + o.radius) }, cylPoint = cvx ? 'far' : 'near';
        var ext = o.bbox ? alongN(o.bbox) : null, at = vdot(f.center, n) - d, eps = 1e-6 * Math.max(1, Math.abs(at));
        if (ext && (at < ext[0] - eps || at > ext[1] + eps)) return;
      } else return;
      if (!(d > 1e-9)) return;
      var cand = { id: e.id, value: sig7(d), overlap: overlap(me.bbox, e.bbox), center: o.center, surface: o.surface,
                   boxCenter: e.bbox ? boxCenter(e.bbox) : null };
      if (cylVals) { cand.values = cylVals; cand.point = cylPoint; cylVals = null; }
      cands.push(cand);
    });
    // 平面在圆柱前(默认对面保持以前的平面, 圆柱是加进来的备选), 同类内重叠优先、再近 → 远
    var isPlane = function (c) { return c.surface === NAMES.planeSurface ? 1 : 0; };
    cands.sort(function (p, q) { return (q.overlap - p.overlap) || (isPlane(q) - isPlane(p)) || (p.value - q.value); });
    out.normal = n;
    out.candidates = cands;
    if (!cands.length) return out;
    // 指定对面(界面切换候选 / Plasticity 里同时选了第二个面)
    var pick = cands[0];
    if (a.opposite !== undefined) {
      pick = cands.filter(function (c) { return c.id === String(a.opposite); })[0];
      if (!pick) throw new Error('not an opposite parallel face of ' + id + ': ' + a.opposite);
    }
    out.kind = 'thickness'; out.value = pick.value; out.opposite = pick.id; out.overlap = pick.overlap; out.op = 'offsetFace'; out.offsetSign = 1;
    // 改对面时: 平面 = 沿自身外法向偏移; 圆柱(孔 / 凸台) = 整体平移(偏移会改半径, 不是位置)
    out.oppositeOp = pick.surface === NAMES.planeSurface ? 'offsetFace' : 'moveFaces';
    return out;
  });

  // N3: 多选面(Plasticity 选择里 ≥ 2 个面)。第一个是平面 → 距离(= dim.distance, 第一个 = 参考面);
  // 全是半径相等的倒圆面 → 一起改圆角半径; 全是半径相等、同凹凸的圆柱 → 一起改半径(用户 2026-11-15: 等半径倒圆不能多选)。
  // args = { faces: [拓扑 id…], units?, angleTol? } → dim.face / dim.distance 同形, 多出 faces: [...]
  def('dim.faces', 'geo.read', function (a) {
    a = a || {};
    var ids = (Array.isArray(a.faces) ? a.faces : []).map(String);
    if (ids.length < 2) throw new Error('dim.faces needs at least two faces');
    var one = function (id) { return API['dim.face'].fn({ face: id, units: a.units, angleTol: a.angleTol }); };
    var first = one(ids[0]);
    if (first.surface === NAMES.planeSurface) {
      return API['dim.distance'].fn({ reference: ids[0], moving: ids.slice(1), units: a.units, angleTol: a.angleTol });
    }
    if (first.kind !== 'filletRadius' && first.kind !== 'radius') {
      throw new Error('first face must be a plane (reference) or all faces fillets / cylinders of one radius (got ' + (first.surface || '?') + ')');
    }
    var tol = 1e-4 * Math.max(1, Math.abs(first.value));
    ids.slice(1).forEach(function (id) {
      var o = one(id);
      if (o.kind !== first.kind) throw new Error('mixed faces: ' + first.kind + ' and ' + o.kind + ' (' + id + ')');
      if (first.kind === 'radius' && o.convex !== first.convex) throw new Error('mixed holes and bosses (' + id + ')');
      if (Math.abs(o.value - first.value) > tol) throw new Error('different radii: ' + first.value + ' and ' + o.value + ' (' + id + ')');
    });
    first.faces = ids;
    return first;
  });

  // N3c: 参考面 → 一组动的面的距离(约定: 第一个选的面 = 参考面, 其余 = 动的面; 用户 2026-11-15 确认)。
  // 参考面必须是平面(法向 n)。动的面的"锚": 有圆柱等带轴线的面 → 轴线(要求与参考面平行, 即轴 ⟂ n; 孔位 / 凸台位置);
  // 否则有与参考面平行的平面 → 其中离参考面最近的那个(两面间距 / 到近壁); 都没有 → 第一个面的中点(近似, 如实标出)。
  // 距离 = 锚到参考平面的法向距离; direction = 参考面指向锚的单位向量。改值 = cmd.exec moveFaces(动的面, direction × (新 − 旧))。
  // args = { reference: 拓扑 id, moving: [拓扑 id…], units?, angleTol? }
  // → { kind: 'distance', value, anchor: 'axis'|'plane'|'center', anchorFace, direction, reference, moving, op: 'moveFaces', units }
  def('dim.distance', 'geo.read', function (a) {
    a = a || {};
    var ref = a.reference !== undefined ? String(a.reference) : null;
    var moving = (Array.isArray(a.moving) ? a.moving : (a.moving !== undefined ? [a.moving] : [])).map(String);
    if (!ref || !moving.length) throw new Error('dim.distance needs {reference, moving: [...]}');
    if (moving.indexOf(ref) >= 0) throw new Error('reference face is also in moving faces');
    var U = unitOf(a), k = U.k;
    var tolRad = ((a.angleTol !== undefined ? Number(a.angleTol) : NAMES.parallelTolDeg) * Math.PI) / 180;
    var cosTol = Math.cos(tolRad), sinTol = Math.sin(tolRad);
    var sigOf = function (id) {
      var m = /^(\d+)[a-z]/.exec(id);
      var sv = m && viewOf('solid', m[1]);
      if (!sv) throw new Error('face not found: ' + id);
      var e = groupEntries(sv, 'face', k).filter(function (x) { return x.id === id; })[0];
      if (!e) throw new Error('face not found in its solid: ' + id);
      return faceSig(e, itemIdOf(sv), k);
    };
    var r = sigOf(ref);
    if (r.surface !== NAMES.planeSurface || !r.normal || !r.center) throw new Error('reference must be a plane (got ' + (r.surface || '?') + ')');
    var ms = moving.map(sigOf);
    var solids = {};
    ms.forEach(function (m) { solids[m.solid] = true; });
    if (Object.keys(solids).length > 1) throw new Error('moving faces must belong to one solid');
    var n = r.normal, anchor = null, kind = null, anchorFace = null;
    ms.some(function (m) {
      // 轴线与参考面平行 = 轴 ⟂ 法向(夹角容差内)
      if (typeof m.radius === 'number' && m.axis && m.axisOrigin && Math.abs(vdot(m.axis, n)) <= sinTol) {
        anchor = m.axisOrigin; kind = 'axis'; anchorFace = m.id; return true;
      }
      return false;
    });
    if (!anchor) {
      // 多个平行平面(如方槽两侧壁): 取离参考面最近的那个(确定、可预期: "到近壁")
      var bestD = Infinity;
      ms.forEach(function (m) {
        if (m.surface !== NAMES.planeSurface || !m.normal || !m.center || Math.abs(vdot(m.normal, n)) < cosTol) return;
        var dd = Math.abs(vdot([m.center[0] - r.center[0], m.center[1] - r.center[1], m.center[2] - r.center[2]], n));
        if (dd < bestD) { bestD = dd; anchor = m.center; kind = 'plane'; anchorFace = m.id; }
      });
    }
    if (!anchor) { anchor = ms[0].center; kind = 'center'; anchorFace = ms[0].id; }
    if (!anchor) throw new Error('cannot locate moving faces');
    var s = vdot([anchor[0] - r.center[0], anchor[1] - r.center[1], anchor[2] - r.center[2]], n);
    var dir = s >= 0 ? n : [-n[0], -n[1], -n[2]];
    // 对称: 每个动的面在参考平面哪一侧(+1 = 与锚同侧, −1 = 另一侧, 0 = 跨在平面上)。两侧都有 → mixed,
    // 界面可选"整体平移"或"以参考面为对称面向两侧移动"。位置取轴线点(带轴线且轴 ⟂ n)或面中点。
    // boxCenters: 各面包围盒中心(文档单位) —— 两步移动时第二步按位置挑面(第一步之后面 id 已变)。
    var eps = 1e-6;
    var sides = ms.map(function (m) {
      var p = (typeof m.radius === 'number' && m.axis && m.axisOrigin && Math.abs(vdot(m.axis, n)) <= sinTol) ? m.axisOrigin : m.center;
      if (!p) return 0;
      var t = vdot([p[0] - r.center[0], p[1] - r.center[1], p[2] - r.center[2]], dir);
      return t > eps ? 1 : t < -eps ? -1 : 0;
    });
    var mixed = sides.indexOf(1) >= 0 && sides.indexOf(-1) >= 0;
    return { kind: 'distance', value: sig7(Math.abs(s)), anchor: kind, anchorFace: anchorFace, direction: dir.map(sig7),
             reference: ref, moving: moving, sides: sides, mixed: mixed,
             boxCenters: ms.map(function (m) { return m.bbox ? boxCenter(m.bbox) : null; }),
             referenceBoxCenter: r.bbox ? boxCenter(r.bbox) : null,
             op: 'moveFaces', units: U.unit };
  });

  // =====================================================================
  // N3d: 面的几何签名 + 按签名重新定位(面 id 每次修改都会变; 尺寸记录 / 选择集 / A9 映射表共用)。
  // 签名 = { solidName, solid, surface, center, normal, radius?, axis?, axisOrigin?, bbox }(文档单位)。
  // 定位: 同名实体(找不到同名 → 全部实体)里同曲面类型、法向(或轴线)平行的面, 取中心最近的; 返回偏差与次优差距。
  // =====================================================================
  function faceSigFull(id, k) {
    var m = /^(\d+)[a-z]/.exec(id);
    var sv = m && viewOf('solid', m[1]);
    if (!sv) throw new Error('face not found: ' + id);
    var e = groupEntries(sv, 'face', k).filter(function (x) { return x.id === id; })[0];
    if (!e) throw new Error('face not found in its solid: ' + id);
    var s = faceSig(e, itemIdOf(sv), k);
    s.solidName = nameOf(sv);
    return s;
  }

  // args = { faces: [拓扑 id…], units? } → { units, faces: [签名 | {id, err}] }
  def('dim.signature', 'geo.read', function (a) {
    a = a || {};
    var U = unitOf(a);
    var faces = (a.faces || []).map(String).map(function (id) {
      try { return faceSigFull(id, U.k); } catch (e) { return { id: id, err: errMsg(e) }; }
    });
    return { units: U.unit, faces: faces };
  });

  // args = { faces: [签名…], units?, tol?(角度, 度; 默认 parallelTolDeg) }
  // → { units, faces: [{ id | null, score(中心偏差, 文档单位), gap(次优 − 最优; null = 唯一), sig(当前签名), err? }] }
  def('geo.resolve', 'geo.read', function (a) {
    a = a || {};
    var U = unitOf(a), k = U.k;
    var cosTol = Math.cos(((a.tol !== undefined ? Number(a.tol) : NAMES.parallelTolDeg) * Math.PI) / 180);
    var all = solidViews();
    var out = (a.faces || []).map(function (sig) {
      if (!sig || !Array.isArray(sig.center)) return { id: null, err: 'bad signature' };
      var pool = all.filter(function (sv) { return sig.solidName && nameOf(sv) === sig.solidName; });
      if (!pool.length) pool = all;
      var best = null, second = null;
      pool.forEach(function (sv) {
        var sid = itemIdOf(sv);
        groupEntries(sv, 'face', k).forEach(function (e) {
          var f = faceSig(e, sid, k);
          if (!f.center || f.surface !== sig.surface) return;
          if (sig.axis && f.axis) { if (Math.abs(vdot(sig.axis, f.axis)) < cosTol) return; }
          else if (sig.normal && f.normal) { if (vdot(sig.normal, f.normal) < cosTol) return; }
          var sc = Math.hypot(f.center[0] - sig.center[0], f.center[1] - sig.center[1], f.center[2] - sig.center[2]);
          if (!best || sc < best.sc) { second = best; best = { f: f, sc: sc, sv: sv }; }
          else if (!second || sc < second.sc) second = { f: f, sc: sc };
        });
      });
      if (!best) return { id: null, err: 'no matching face (solid ' + (sig.solidName || '?') + ', ' + sig.surface + ')' };
      best.f.solidName = nameOf(best.sv);
      return { id: best.f.id, score: sig7(best.sc), gap: second ? sig7(second.sc - best.sc) : null, sig: best.f };
    });
    return { units: U.unit, faces: out };
  });

  // N3d: 一条尺寸记录 → 一次调用里按签名定位 + 读当前尺寸(不经过 Plasticity 选择, 改值不用等选择回传)。
  // args = { mode: 'face'|'distance', faces: [签名…](distance: 参考在前), opposite?: 签名, units? }
  // → { ids, opposite: id|null, oppositeLost?: true(记录的对面找不到 / 不再是候选 → 用默认对面), worst(最大中心偏差), info(= dim.face / dim.distance) }
  def('dim.record', 'geo.read', function (a) {
    a = a || {};
    var sigs = Array.isArray(a.faces) ? a.faces : [];
    if (!sigs.length) throw new Error('dim.record needs {faces: [signature…]}');
    var n = sigs.length;
    var res = API['geo.resolve'].fn({ faces: a.opposite ? sigs.concat([a.opposite]) : sigs, units: a.units }).faces;
    var lost = res.slice(0, n).filter(function (r) { return !r.id; })[0];
    if (lost) throw new Error('face not found in the current model: ' + (lost.err || '?'));
    var ids = res.slice(0, n).map(function (r) { return r.id; });
    var worst = res.reduce(function (m, r) { return typeof r.score === 'number' ? Math.max(m, r.score) : m; }, 0);
    var out = { ids: ids, opposite: null, worst: worst };
    if (a.mode === 'distance') {
      if (n < 2) throw new Error('distance record needs a reference and at least one moving face');
      out.info = API['dim.distance'].fn({ reference: ids[0], moving: ids.slice(1), units: a.units });
      return out;
    }
    if (n > 1) { out.info = API['dim.faces'].fn({ faces: ids, units: a.units }); return out; }   // 多个等半径倒圆 / 圆柱
    var opp = a.opposite && res[n] && res[n].id;
    if (opp) {
      try { out.info = API['dim.face'].fn({ face: ids[0], opposite: opp, units: a.units }); out.opposite = opp; }
      catch (e) { out.oppositeLost = true; }
    } else if (a.opposite) out.oppositeLost = true;
    if (!out.info) { out.info = API['dim.face'].fn({ face: ids[0], units: a.units }); out.opposite = out.info.opposite || null; }
    return out;
  });

  // N3e: 本体测量(Plasticity 自己的 Distance 测量, 大纲里可见、随文件保存)。
  // 两个面的面心吸附点之间建一个两点测量, 量的方向 = normal(默认第一个面的法向) → 标签值 = 两面法向距离;
  // 名字写进 nodes 表(大纲显示的就是它)。同名测量已存在 → 不重复建(exists)。
  // args = { faces: [拓扑 id, 拓扑 id], name, normal?: [x,y,z] } | { rename: { from, to } } | { find: 名字 }
  // → { created | exists | renamed | found: bool, name }
  function measureRecords() {
    var snap = invokeOn('meas.snapshot', []);
    var list = walk(snap, NAMES.measure.list);
    return list === MISS || !list ? [] : Array.from(list);
  }
  function measureNamed(name) {
    return measureRecords().filter(function (m) {
      try { return invokeOn('nodes.getName', [invokeOn('nodes.item2key', [m])]) === name; } catch (e) { return false; }
    })[0] || null;
  }
  // 面 → 它的面心吸附对象(SnapScene 缓存里与面中点重合的 FaceCenterPointSnap) + 位置(米)
  function faceCenterSnap(faceId) {
    var MS = NAMES.measure;
    var m = /^(\d+)[a-z]/.exec(String(faceId));
    var sv = m && viewOf('solid', m[1]);
    if (!sv) throw new Error('face not found: ' + faceId);
    var sig = faceSigFull(String(faceId), 1);
    if (!sig.center) throw new Error('no midpoint for face ' + faceId);
    var entry = snapEntryOf(sv);
    if (!entry || typeof entry[MS.snapLookup] !== 'function') throw new Error('no snap data for the solid of ' + faceId);
    var best = null, bd = Infinity;
    for (var i = 0; i < MS.lookupMax; i++) {
      var s;
      try { s = entry[MS.snapLookup](i); } catch (e) { break; }
      if (!s) break;
      if (!s.constructor || s.constructor.name !== MS.faceSnapClass) continue;
      var p = s[MS.snapPos];
      var d = Math.hypot(p.x - sig.center[0], p.y - sig.center[1], p.z - sig.center[2]);
      if (d < bd) { bd = d; best = s; }
    }
    if (!best || bd > MS.snapTolM) throw new Error('no face-center snap point for ' + faceId + (best ? ' (nearest ' + bd + ' m)' : ''));
    return { snap: best, normal: sig.normal };
  }
  def('dim.measure', 'measure.write', function (a) {
    a = a || {};
    var Cls = factoryCommandClass();
    var run = function (build) {
      var cmd = new Cls(editor(), build);
      return runCommand(cmd, NAMES.cmdTimeoutMs).then(function (how) {
        if (cmd.plaErr || how !== 'ended') throw new Error(cmd.plaErr || how);
        return cmd.plaResult;
      });
    };
    if (a.find !== undefined) return Promise.resolve({ found: !!measureNamed(String(a.find)), name: String(a.find) });
    if (a.rename) {
      var from = String(a.rename.from || ''), to = String(a.rename.to || '');
      if (!from || !to) throw new Error('rename needs {from, to}');
      var rec = measureNamed(from);
      if (!rec) return Promise.resolve({ renamed: false, name: from });
      return run(function () { invokeOn('nodes.setName', [invokeOn('nodes.item2key', [rec]), to]); return null; })
        .then(function () { return { renamed: true, name: to }; });
    }
    var name = String(a.name || '');
    var faces = Array.isArray(a.faces) ? a.faces.map(String) : [];
    if (!name || faces.length !== 2) throw new Error('dim.measure needs {faces: [id, id], name}');
    if (measureNamed(name)) return Promise.resolve({ exists: true, name: name });
    if (!W[CFG.factoriesGlobal]) throw new Error(CFG.needFactoriesErr);
    var Fac = need('fac.p2pMeasure');
    var A = faceCenterSnap(faces[0]), B = faceCenterSnap(faces[1]);
    var n = Array.isArray(a.normal) && a.normal.length === 3 ? a.normal : A.normal;
    if (!n) throw new Error('no measuring direction (first face has no normal)');
    var MS = NAMES.measure;
    return run(function (cmd) {
      var f = new Fac(cmd.editor);
      f[NAMES.facResource](cmd);
      f[MS.p1](A.snap[MS.snapPos].clone(), A.snap);
      f[MS.p2](B.snap[MS.snapPos].clone(), B.snap);
      var V = A.snap[MS.snapPos].constructor;
      f[MS.orientation].setFromUnitVectors(new V(MS.axis[0], MS.axis[1], MS.axis[2]), new V(n[0], n[1], n[2]).normalize());
      return Promise.resolve(f[NAMES.facCommit]()).then(function (m) {
        invokeOn('nodes.setName', [invokeOn('nodes.item2key', [m]), name]);
        return m;
      });
    }).then(function () { return { created: true, name: name }; });
  }, { async: true });

  // N4 打孔。在平面上 at 处(默认面中点; 会投影到面所在平面)打孔, 一次原子执行(失败整体撤回):
  // 孔 = 圆柱 subtract(轴 = 入口面法向的反向; 通孔深度 = 实体包围盒对角线 + 1); 柱形沉头 = 再减一个大而浅的圆柱;
  // 锥形沉头(90°) = 孔口边倒角 (cskD − d) / 2(第二步按位置挑边: 孔口圆的包围盒中心就是 at)。
  // (参数与回包见 hole.make 定义处)
  // 平面内的两条偏移轴: u = 与法向最不平行的世界轴(法向近 Z → X; 近 X → Y; 否则 X)投影到平面, v = n × u。
  // 返回 { u, v, uName, vName }(名字 = 与之最接近的世界轴, 带符号, 界面标签用)
  function planeAxes(n) {
    var base = Math.abs(n[2]) > 0.9 ? [1, 0, 0] : Math.abs(n[0]) > 0.9 ? [0, 1, 0] : [1, 0, 0];
    var d = vdot(base, n);
    var u = vnorm([base[0] - d * n[0], base[1] - d * n[1], base[2] - d * n[2]]);
    var v = vcross(n, u);
    var nameOf = function (w) { var i = [0, 1, 2].sort(function (p, q) { return Math.abs(w[q]) - Math.abs(w[p]); })[0]; return (w[i] < 0 ? '-' : '') + 'XYZ'.charAt(i); };
    return { u: u.map(sig7), v: v.map(sig7), uName: nameOf(u), vName: nameOf(v) };
  }

  // =====================================================================
  // N5: 资产(asset pack)。asset.list = 已登记的 pack 与里面的资产; asset.place = 走 Plasticity 自己的拖放导入
  // (同手工拖, 带资产引用), 放置时把目标点 / 朝向换成给定的。know-how/asset-place.md。
  // =====================================================================
  function packList() {
    var D = NAMES.assetDnd;
    return Array.from(invokeOn('assets.packs', []) || []).map(function (p) { return { path: p[D.pack.path] }; });
  }

  function loadPack(want) {
    var D = NAMES.assetDnd, packs = packList();
    if (!packs.length) throw new Error('no asset packs registered in Plasticity (Assets panel)');
    var low = String(want || '').toLowerCase();
    var p = want ? packs.find(function (x) { return x.path === want || x.path.toLowerCase().indexOf(low) >= 0; }) : packs[0];
    if (!p) throw new Error('no pack matches ' + want + ' (have: ' + packs.map(function (x) { return x.path; }).join('; ') + ')');
    return Promise.resolve(invokeOn('assets.load', [p.path])).then(function (pk) { return { path: p.path, pack: pk }; });
  }

  def('asset.list', 'asset.place', function (a) {
    a = a || {};
    var D = NAMES.assetDnd, k = unitOf(a).k, packs = packList();
    var wanted = a.pack ? [a.pack] : packs.map(function (p) { return p.path; });
    return wanted.reduce(function (pr, w) {
      return pr.then(function (acc) {
        return loadPack(w).then(function (r) {
          acc.push({ path: r.path, assets: (r.pack[D.pack.assets] || []).map(function (x) {
            var bb = x[D.asset.bbox] || [];
            return { name: x[D.asset.name], id: x[D.asset.id], folder: (x[D.asset.folder] || []).join('/'), size: bb.map(function (v) { return sig7(v / k); }) };
          }) });
          return acc;
        }, function (e) { acc.push({ path: w, err: errMsg(e) }); return acc; });
      });
    }, Promise.resolve([])).then(function (list) { return { units: unitOf(a).unit, packs: list }; });
  }, { async: true });

  // args = { asset: 名字(不分大小写)或资产 id, pack?: 路径或其中一段, at: [x,y,z](pivot 落点, 文档单位), normal?: [x,y,z](资产 +Z 指向, 默认 +Z = 面外法向),
  //          rotate?: 度(绕 normal), units? }
  // → { asset, items: [{kind, id, name, bbox}], undoSteps }
  def('asset.place', 'asset.place', function (a) {
    a = a || {};
    var D = NAMES.assetDnd, U = unitOf(a), k = U.k;
    if (W.document && W.document.visibilityState === 'hidden') throw new Error('asset.place needs a visible Plasticity window (drag-and-drop placement)');
    if (!Array.isArray(a.at) || a.at.length !== 3) throw new Error('at must be [x,y,z]');
    var at = a.at.map(function (x) { return Number(x) * k; });
    var n = vnorm((Array.isArray(a.normal) && a.normal.length === 3 ? a.normal : D.baseNormal).map(Number));
    var rot = (Number(a.rotate) || 0) * Math.PI / 180;
    var Place = need('fac.place');
    var canvas = (viewportElement() || W.document).querySelector('canvas');
    if (!canvas) throw new Error('viewport canvas not found');
    var mark = markNow();
    var before = new Set(Array.from(need('geo.items') || []).map(function (it) { return it[NAMES.lookupViewKey]; }));
    // 轮询; cond 抛错 → 拒绝(setTimeout 回调里的异常不会传到 Promise, 不接住就永远挂着)
    var wait = function (cond, ms, what) {
      return new Promise(function (res, rej) {
        var t0 = Date.now();
        (function poll() {
          var v;
          try { v = cond(); } catch (e) { return rej(e); }
          if (v) return res(v);
          if (Date.now() - t0 > ms) return rej(new Error((what || 'wait') + ' timed out'));
          setTimeout(poll, 50);
        })();
      });
    };
    var fresh = function () { return Array.from(need('geo.items') || []).map(function (it) { return it[NAMES.lookupViewKey]; }).filter(function (v) { return v && !before.has(v); }); };
    var report = { asset: null };
    var overridden = false;
    return loadPack(a.pack).then(function (r) {
      var key = String(a.asset || '').toLowerCase();
      var asset = (r.pack[D.pack.assets] || []).find(function (x) { return String(x[D.asset.name]).toLowerCase() === key || x[D.asset.id] === a.asset; });
      if (!asset) throw new Error('asset ' + a.asset + ' not in ' + r.path + ' (have: ' + (r.pack[D.pack.assets] || []).map(function (x) { return x[D.asset.name]; }).join(', ') + ')');
      report.asset = asset[D.asset.name];
      var data = JSON.stringify({ id: asset[D.asset.id], pivot: asset[D.asset.pivot], packId: r.pack[D.pack.id], packRevision: r.pack[D.pack.revision], packAbsolutePath: r.path, contentHash: asset[D.asset.hash] });
      try { invokeOn('exec.cancel'); } catch (e) {}
      // 要等**新的**导入命令(看正在执行的命令; 上一次没收尾的那条取消后 lastCommand 还指着它)
      var prev = get('exec.activeCmd');
      var rc = canvas.getBoundingClientRect(), x = rc.left + rc.width / 2, y = rc.top + rc.height / 2;
      var dt = new W.DataTransfer();
      dt.setData(D.mime, data);
      ['dragenter', 'dragover', 'drop'].forEach(function (t) {
        canvas.dispatchEvent(new W.DragEvent(t, { bubbles: true, cancelable: true, composed: true, clientX: x, clientY: y, dataTransfer: dt }));
      });
      // 命令对象本身是 thenable(有 then): 直接 resolve 它会让链等命令结束 —— 而命令在等我们放置 = 死锁(实测 2026-11-18)→ 包一层
      return wait(function () {
        var c = get('exec.activeCmd');
        return c && c !== prev && c.constructor && c.constructor.name === D.importCmd && (c[D.promises] || []).length ? { cmd: c } : null;
      }, D.startWaitMs, 'import command');
    }).then(function (box) {
      var cmd = box.cmd;
      // 放置: 只在这一次 commit 里把目标换掉(PlaceFactory 自己没有 commit, 挂一个本实例原型上的再删掉)
      var P = Place.prototype, base = P[NAMES.facCommit];
      P[NAMES.facCommit] = function () {
        var V = this[D.dest].constructor;
        this[D.dest].set(at[0], at[1], at[2]);
        var q = this[D.destQ];
        q.setFromUnitVectors(new V(D.baseNormal[0], D.baseNormal[1], D.baseNormal[2]), new V(n[0], n[1], n[2]));
        if (rot) { var qr = q.clone().setFromAxisAngle(new V(n[0], n[1], n[2]), rot); q.premultiply(qr); }
        return base.apply(this, arguments);
      };
      overridden = true;
      // 放置拾取器要先收到一次指针移动才有"当前点", 否则 finish() 什么也不做(实测 2026-11-18)
      var rc2 = canvas.getBoundingClientRect();
      canvas.dispatchEvent(new W.PointerEvent('pointermove', { bubbles: true, cancelable: true, composed: true, clientX: rc2.left + rc2.width / 2, clientY: rc2.top + rc2.height / 2, pointerId: 1, pointerType: 'mouse', isPrimary: true }));
      return new Promise(function (r2) { setTimeout(r2, D.moveSettleMs); }).then(function () {
        cmd[D.promises].slice().forEach(function (p) { if (typeof p[D.finish] === 'function') { try { p[D.finish](); } catch (e) {} } });
        return wait(function () { return undoStack().length > mark.depth; }, D.placeWaitMs);
      }).then(function () {
        delete P[NAMES.facCommit]; overridden = false;
        return new Promise(function (r2) { setTimeout(r2, 150); });
      });
    }).then(function () {
      var made = fresh();
      report.items = made.map(function (v) { return describeMade(v, k); });
      return report;
    }).then(function (rep) {
      rep.undoSteps = undoStack().length - mark.depth;
      return rep;
    }, function (e) {
      if (overridden) { try { delete Place.prototype[NAMES.facCommit]; } catch (x) {} }
      // 半途失败: 撤到放之前(导入还在等放置时先取消命令)
      try { invokeOn('exec.cancel'); } catch (x) {}
      return rollbackTo(mark).then(function () { throw e; }, function () { throw e; });
    });
  }, { async: true });

  // N5b 建库(实测 2026-11-18, know-how/asset-place.md): 资产 = 标了资产标记的组**或单个实体**; 名字和 pivot 在标记那一刻定下来
  // (之后改名 / setPivot 不进 pack)→ 先改名 + setPivot 再标。ToggleAssetCommand 作用于当前选择, 标完 Plasticity 会自动起一个
  // MoveItemCommand(agent automatic), 取消即可。pivot = {position: Vector3(米), orientation: Quaternion}(给数组 getPivot 照样回, 但导出静默失败); 不进撤销栈。
  // args = { solid, name, pivot: [x,y,z](文档单位), orientation?: [x,y,z,w](默认不转 = 资产 +Z 同世界 +Z), units? } → { solid, name, asset: true }
  def('asset.mark', 'asset.lib', function (a) {
    a = a || {};
    var k = unitOf(a).k;
    var v = viewOf('solid', a.solid);
    if (!v) throw new Error('solid not found: ' + a.solid);
    if (!a.name) throw new Error('name is required');
    if (!Array.isArray(a.pivot) || a.pivot.length !== 3) throw new Error('pivot must be [x,y,z]');
    var key = invokeOn('nodes.item2key', [v]);
    invokeOn('nodes.setName', [key, String(a.name)]);
    var PC = NAMES.assetDnd.pivotCtors, Vc = v[PC.vec] && v[PC.vec].constructor, Qc = v[PC.quat] && v[PC.quat].constructor;
    if (!Vc || !Qc) throw new Error('asset.mark: no Vector3 / Quaternion class on the solid view');
    var o = Array.isArray(a.orientation) && a.orientation.length === 4 ? a.orientation.map(Number) : [0, 0, 0, 1];
    invokeOn('nodes.setPivot', [key, { position: new Vc(Number(a.pivot[0]) * k, Number(a.pivot[1]) * k, Number(a.pivot[2]) * k), orientation: new Qc(o[0], o[1], o[2], o[3]) }]);
    if (invokeOn('nodes.isAsset', [key])) return { solid: itemIdOf(v), name: String(a.name), asset: true, already: true };
    var s = need('sel.selected');
    var prev = [];
    NAMES.exportStep.selSets.forEach(function (k2) { var set = s[k2]; if (set && typeof set.forEach === 'function') set.forEach(function (x) { prev.push(x); }); });
    try { invokeOn('exec.cancel'); } catch (e) {}
    invokeOn('sel.removeAll', [], s);
    invokeOn('sel.add', [v], s);
    var Toggle = need('cmd.toggleAsset');
    var tg = new Toggle(editor());
    invokeOn('ed.enqueue', [tg]);
    // 选上实体后 Plasticity 会自动起 MoveItemCommand(agent automatic), 排在它后面的标记命令就一直等 —— 不是我们的标记命令就取消;
    // 标上以后再安静 500ms(没有活动命令)才算完(实测 2026-11-18: 第二件起必卡)
    return new Promise(function (res, rej) {
      var t0 = Date.now(), tOk = 0;
      (function poll() {
        var ok = false;
        try { ok = invokeOn('nodes.isAsset', [key]); } catch (e) { return rej(e); }
        var act = get('exec.activeCmd');
        if (act && act !== tg) { try { invokeOn('exec.cancel'); } catch (e) {} }
        if (ok) {
          if (!tOk) tOk = Date.now();
          if (!act && Date.now() - tOk > 500) {
            try { invokeOn('sel.removeAll', [], s); prev.forEach(function (x) { invokeOn('sel.add', [x], s); }); } catch (e) {}
            return res({ solid: itemIdOf(v), name: String(a.name), asset: true });
          }
        } else if (Date.now() - t0 > 5000) return rej(new Error('asset.mark: not marked within 5 s'));
        setTimeout(poll, 100);
      })();
    });  }, { async: true });

  // N5b: 把文档里所有资产导出成一个 pack(存盘对话框要先在主进程武装, 见 pla-bridge export_pack)。导出会顺手把 pack 登记进 Assets 面板。
  // 不覆盖已有文件(实测: 目标已存在时什么也不写)→ 调用方导到新文件名再换上。
  // args = { expect?: 应有的资产数 —— 文档里还有别的资产就拒绝(否则用户自己的资产会混进库里)} → { started: true, assets }
  def('asset.exportPack', 'asset.lib', function (a) {
    var n = Array.from(invokeOn('nodes.findAllAssets', []) || []).length;
    if (a && a.expect !== undefined && n !== Number(a.expect)) throw new Error('the document holds ' + n + ' assets, expected ' + a.expect + ' (build libraries in a document without other assets)');
    try { invokeOn('exec.cancel'); } catch (e) {}
    var Exp = need('cmd.exportAssetPack');
    invokeOn('ed.enqueue', [new Exp(editor())]);
    return { started: true, assets: n };
  });

  // N5b: 登记 / 撤销登记 pack(Assets 面板里的清单)。add 收**数组**(给字符串会被当成一个个字符登记 —— 实测踩过), 新的排最前、去重。
  function packPaths(a) {
    var ps = (a && Array.isArray(a.paths)) ? a.paths : [];
    if (!ps.length) throw new Error('paths must be a non-empty array of .plasticitypack paths');
    ps.forEach(function (p) { if (typeof p !== 'string' || !/\.plasticitypack$/i.test(p) || !/^([a-zA-Z]:\\|\\\\|\/)/.test(p)) throw new Error('not an absolute .plasticitypack path: ' + p); });
    return ps;
  }
  def('asset.register', 'asset.lib', function (a) {
    var ps = packPaths(a);
    var r = invokeOn('assets.add', [ps.slice()]);
    return Promise.resolve(r).then(function () { return { packs: packList().map(function (p) { return p.path; }) }; });
  }, { async: true });
  def('asset.unregister', 'asset.lib', function (a) {
    var ps = packPaths(a);
    ps.forEach(function (p) { invokeOn('assets.remove', [p]); });
    return { packs: packList().map(function (p) { return p.path; }) };
  });
  // N5: 螺钉改长度(用户 2026-11-18: pack 里的螺钉统一 10mm, 放进来后**只移动杆端的面**, 不整体缩放)。
  // 杆端的面 = 沿 axis 投影后贴着最远端、且整个落在端部区(zone)里的面: 端面 + 倒角锥; 整体沿 axis 平移 delta(moveFaces), 杆的圆柱面跟着伸缩。
  // args = { solid, axis: [x,y,z](朝杆端), delta: 文档单位(+ = 加长), zone?: 端部区长度(默认 min(2.5, 0.3 × 总长)), units? }
  // → { solid: 结果实体 id, moved: [面 id], delta }
  def('part.stretch', 'cmd.exec', function (a) {
    a = a || {};
    var U = unitOf(a), k = U.k;
    var sv = viewOf('solid', a.solid);
    if (!sv) throw new Error('solid not found: ' + a.solid);
    if (!Array.isArray(a.axis) || a.axis.length !== 3) throw new Error('axis must be [x,y,z] (toward the tip)');
    var ax = vnorm(a.axis.map(Number)), delta = Number(a.delta);
    if (!isFinite(delta)) throw new Error('delta must be a number');
    var sid = itemIdOf(sv);
    if (Math.abs(delta) < 1e-9) return { solid: sid, moved: [], delta: 0 };
    var span = function (b) {
      var lo = Infinity, hi = -Infinity;
      for (var i = 0; i < 8; i++) { var t = vdot([(i & 1 ? b[1] : b[0])[0], (i & 2 ? b[1] : b[0])[1], (i & 4 ? b[1] : b[0])[2]], ax); lo = Math.min(lo, t); hi = Math.max(hi, t); }
      return [lo, hi];
    };
    var ents = groupEntries(sv, 'face', k).filter(function (e) { return e.bbox; }).map(function (e) { return { id: e.id, s: span(e.bbox) }; });
    if (!ents.length) throw new Error('solid has no faces: ' + a.solid);
    var far = Math.max.apply(null, ents.map(function (e) { return e.s[1]; })), near = Math.min.apply(null, ents.map(function (e) { return e.s[0]; }));
    var zone = a.zone !== undefined ? Number(a.zone) : Math.min(2.5, 0.3 * (far - near)), eps = 1e-6 * Math.max(1, Math.abs(far));
    var tip = ents.filter(function (e) { return e.s[1] >= far - eps && e.s[0] >= far - zone - eps; });
    if (!tip.length) throw new Error('no tip faces found');
    // 缩短不能吃到头
    if (delta < 0 && -delta >= (far - near) - zone) throw new Error('cannot shorten by ' + (-delta) + ' (part is ' + sig7(far - near) + ' long)');
    var move = ax.map(function (x) { return x * delta; });
    return API['cmd.exec'].fn({ atomic: true, ops: [{ op: 'moveFaces', id: 's0', faces: tip.map(function (e) { return e.id; }), move: move }], units: U.unit }).then(function (r) {
      if (r.failed) throw new Error('stretch: ' + r.failed.err + (r.rolledBack !== undefined ? ' — rolled back' : ''));
      var it = r.results[0] && r.results[0].items && r.results[0].items[0];
      return { solid: it && it.id !== undefined ? it.id : sid, moved: tip.map(function (e) { return e.id; }), delta: sig7(delta) };
    });
  }, { async: true });
  // N4b 打孔位置: 选择里的草图点 → 文档单位坐标(用户 2026-11-15: 用草图端点定孔位, 选几个点打几个孔)。
  // curveVertex = 曲线端点(lookupTopologyItemById → Vertex.GetPoint); curve = 整条曲线: 有端点 → 各端点, 闭合(圆等)→ 包围盒中心(= 圆心)。
  function sketchPoints(refs, k) {
    var HK = NAMES.holePointKinds, Wn = NAMES.wire, out = [];
    (refs || []).forEach(function (r) {
      var kind = String(r && r.kind), id = String(r && r.id);
      if (kind === HK.vertex) {
        var t = invokeOn('geo.lookupTopo', [id]);
        var m = t && t[NAMES.lookupModelKey];
        if (!m) throw new Error('curve point not found: ' + id);
        var p = m[NAMES.geoApi.point]();
        out.push([p.x / k, p.y / k, p.z / k]);
      } else if (kind === HK.curve) {
        var v = viewOf('curve', id);
        if (!v) throw new Error('curve not found: ' + id);
        var vg = v[Wn.vertices], pos = vg ? walk(vg, Wn.vertexPositions) : MISS;
        if (vg && vg.length && pos !== MISS && pos) {
          for (var i = 0; i < vg.length; i++) out.push([pos[i * 3] / k, pos[i * 3 + 1] / k, pos[i * 3 + 2] / k]);
        } else {
          var bx = v[Wn.segments] && v[Wn.segments][NAMES.groupBboxes];
          if (!bx || !bx.length) throw new Error('curve has no geometry: ' + id);
          var mn = [Infinity, Infinity, Infinity], mx = [-Infinity, -Infinity, -Infinity];
          for (var j = 0; j + 5 < bx.length; j += 6) for (var q = 0; q < 3; q++) { mn[q] = Math.min(mn[q], bx[j + q]); mx[q] = Math.max(mx[q], bx[j + 3 + q]); }
          out.push([(mn[0] + mx[0]) / 2 / k, (mn[1] + mx[1]) / 2 / k, (mn[2] + mx[2]) / 2 / k]);
        }
      } else throw new Error('hole points must be curve points or curves (got ' + kind + ')');
    });
    return out;
  }

  // args = { face, points?: [{kind: curveVertex|curve, id}](省略 = at / 面中点 + offset), stack?: [实体 id…](组合孔, 按选择顺序),
  //          type: simple|counterbore|countersink, d(入口实体 / 中间实体的孔径), tapD?(组合孔: 最后一个实体的孔径),
  //          depth?(省略 = 通孔; 组合孔 = 最后一个实体里的深度), cbD?, cbDepth?, cskD?, at?, offset?, units? }
  // 组合孔(用户 2026-11-15): 沉头只在入口面所在实体; 入口实体与中间实体 = d 的贯穿孔; 最后一个实体 = tapD(盲孔深度从它的入口算)。
  // → { holes: [{ at, t0?, faces: { entry, wall, bottom?, cbWall?, cbFloor?, csk?, tapWall?, tapEntry? } }], solids, through, depth, normal, units }
  def('hole.make', 'cmd.exec', function (a) {
    a = a || {};
    var U = unitOf(a), k = U.k;
    var num = function (x, what) { var n = Number(x); if (!(n > 0)) throw new Error(what + ' must be > 0'); return n; };
    var type = a.type || 'simple';
    if (['simple', 'counterbore', 'countersink'].indexOf(type) < 0) throw new Error('type must be simple / counterbore / countersink');
    var d = num(a.d, 'd');
    var f = faceSigFull(String(a.face || ''), k);
    if (f.surface !== NAMES.planeSurface || !f.normal || !f.center) throw new Error('holes go on a plane face (got ' + (f.surface || '?') + ')');
    var n = f.normal, c = f.center;
    // 位置: 草图点; 没有就 at / 面中点 + offset(沿 planeAxes)
    var pts = sketchPoints(a.points, k);
    if (!pts.length) {
      var at0 = Array.isArray(a.at) && a.at.length === 3 ? a.at.map(Number) : c.slice();
      if (Array.isArray(a.offset) && a.offset.length === 2) {
        var ax2 = planeAxes(n), ou = Number(a.offset[0]) || 0, ov = Number(a.offset[1]) || 0;
        at0 = [at0[0] + ax2.u[0] * ou + ax2.v[0] * ov, at0[1] + ax2.u[1] * ou + ax2.v[1] * ov, at0[2] + ax2.u[2] * ou + ax2.v[2] * ov];
      }
      pts = [at0];
    }
    pts = pts.map(function (p) { var o = vdot([p[0] - c[0], p[1] - c[1], p[2] - c[2]], n); return [p[0] - n[0] * o, p[1] - n[1] * o, p[2] - n[2] * o]; });
    // 实体: 入口(面所在) + 组合孔的其余实体(按顺序, 最后一个 = 螺纹 / 底孔)
    var A = itemIdOf(viewOf('solid', /^(\d+)/.exec(String(a.face))[1]));
    var stack = (Array.isArray(a.stack) ? a.stack : []).map(String).filter(function (s) { return s !== String(A); });
    stack.forEach(function (s) { if (!viewOf('solid', s)) throw new Error('solid not found: ' + s); });
    // dry: 只算孔位 + 第一个孔位上各实体沿孔轴的入口 / 出口深度(包围盒投影; 打孔面板算螺丝长度用), 不打孔
    if (a.dry) {
      var ax0 = n.map(function (x) { return -x; }), p0 = pts[0];
      var layers = [A].concat(stack).map(function (s) {
        var b = bboxOf(viewOf('solid', s), k), lo = Infinity, hi = -Infinity;
        for (var i = 0; i < 8; i++) { var q = [(i & 1 ? b[1] : b[0])[0], (i & 2 ? b[1] : b[0])[1], (i & 4 ? b[1] : b[0])[2]]; var t = vdot([q[0] - p0[0], q[1] - p0[1], q[2] - p0[2]], ax0); lo = Math.min(lo, t); hi = Math.max(hi, t); }
        return { solid: s, t0: sig7(Math.max(0, lo)), t1: sig7(hi) };
      });
      return { dry: true, holes: pts.map(function (p) { return { at: p.map(sig7) }; }), normal: n.map(sig7), solid: A, layers: layers, units: U.unit };
    }
    var last = stack.length ? stack[stack.length - 1] : null, mids = stack.slice(0, -1);
    var tapD = last ? num(a.tapD, 'tapD') : null;
    var boxes = [A].concat(stack).map(function (s) { return bboxOf(viewOf('solid', s), k); });
    var mn = [Infinity, Infinity, Infinity], mx = [-Infinity, -Infinity, -Infinity];
    boxes.forEach(function (b) { for (var q = 0; q < 3; q++) { mn[q] = Math.min(mn[q], b[0][q]); mx[q] = Math.max(mx[q], b[1][q]); } });
    var long = Math.hypot(mx[0] - mn[0], mx[1] - mn[1], mx[2] - mn[2]) + 1;
    var through = a.depth === undefined || a.depth === null;
    var depth = through ? long : num(a.depth, 'depth');
    var axis = n.map(function (x) { return x === 0 ? 0 : -x; });   // 不出 −0
    var cbD, cbDepth, cskD;
    if (type === 'counterbore') {
      cbD = num(a.cbD, 'cbD'); cbDepth = num(a.cbDepth, 'cbDepth');
      if (cbD <= d) throw new Error('counterbore diameter must be larger than the hole');
    } else if (type === 'countersink') {
      cskD = num(a.cskD, 'cskD');
      if (cskD <= d) throw new Error('countersink diameter must be larger than the hole');
    }
    // 实体在孔轴上的入口深度(相对孔口, 沿 axis): 包围盒 8 角投影的最小值(≥ 0; 叠放的板材是准的)
    var entryDepth = function (s, p) {
      var b = bboxOf(viewOf('solid', s), k), t = Infinity;
      for (var i = 0; i < 8; i++) { var q = [(i & 1 ? b[1] : b[0])[0], (i & 2 ? b[1] : b[0])[1], (i & 4 ? b[1] : b[0])[2]]; t = Math.min(t, vdot([q[0] - p[0], q[1] - p[1], q[2] - p[2]], axis)); }
      return Math.max(0, t);
    };
    // 步骤: 每步只减一个实体; ref = 该实体当前的引用(上一步产物 '$sN')
    var ops = [], meta = [], ref = {};
    [A].concat(stack).forEach(function (s) { ref[s] = s; });
    var step = function (o, s) { o.id = 's' + ops.length; if (o.op === 'cylinder') { o.bool = 'subtract'; o.targets = [ref[s]]; } ops.push(o); meta.push(s); ref[s] = '$' + o.id; };
    var cyl = function (p, r, h) { return { op: 'cylinder', center: p, axis: axis, radius: r, height: h }; };
    var holes = pts.map(function (p) {
      var hole = { at: p };
      if (!last) step(cyl(p, d / 2, depth), A);
      else {
        step(cyl(p, d / 2, long), A);
        mids.forEach(function (m) { step(cyl(p, d / 2, long), m); });
        hole.t0 = entryDepth(last, p);
        step(cyl(p, tapD / 2, through ? long : hole.t0 + depth), last);
      }
      if (type === 'counterbore') step(cyl(p, cbD / 2, cbDepth), A);
      if (type === 'countersink') step({ op: 'chamfer', edges: [{ near: p, tol: d / 4 }], distance: (cskD - d) / 2 }, A);
      return hole;
    });
    return API['cmd.exec'].fn({ atomic: true, ops: ops, units: U.unit }).then(function (r) {
      if (r.failed) throw new Error('step ' + r.failed.index + ' (' + r.failed.op + '): ' + r.failed.err + (r.rolledBack !== undefined ? ' — rolled back' : ''));
      // 各实体的最终产物
      var fin = {};
      meta.forEach(function (s, i) { var it = r.results[i] && r.results[i].items[0]; if (it && it.id !== undefined) fin[s] = it.id; });
      var sigCache = {};
      var sigsOf = function (s) {
        var id = fin[s] !== undefined ? fin[s] : s;
        if (sigCache[id]) return sigCache[id];
        var nv = viewOf('solid', id);
        if (!nv) throw new Error('hole made but the result solid was not found');
        return (sigCache[id] = groupEntries(nv, 'face', k).map(function (e) { var sg = faceSig(e, id, k); sg.solidName = nameOf(nv); return sg; }));
      };
      var tol = Math.max(1e-3, d * 1e-3);
      holes.forEach(function (h) {
        var at = h.at;
        var distToAxis = function (s) {
          var w = [at[0] - s.axisOrigin[0], at[1] - s.axisOrigin[1], at[2] - s.axisOrigin[2]], al = vdot(w, s.axis);
          return Math.hypot(w[0] - al * s.axis[0], w[1] - al * s.axis[1], w[2] - al * s.axis[2]);
        };
        var cylOf = function (s, rr) { return sigsOf(s).filter(function (g) { return typeof g.radius === 'number' && g.axis && g.axisOrigin && g.surface !== NAMES.coneSurface && Math.abs(g.radius - rr) < tol && Math.abs(Math.abs(vdot(g.axis, n)) - 1) < 1e-4 && distToAxis(g) < tol; })[0]; };
        // 朝外(= +n)的平面, 中心在孔口下方 hh 处(入口 / 孔底 / 沉头台阶), 取离孔轴最近的
        var floorAt = function (s, hh) { return sigsOf(s).filter(function (g) { return g.surface === NAMES.planeSurface && g.normal && vdot(g.normal, n) > 0.9999 && Math.abs(vdot([at[0] - g.center[0], at[1] - g.center[1], at[2] - g.center[2]], n) - hh) < tol; }).sort(function (p, q) { return Math.hypot(p.center[0] - at[0], p.center[1] - at[1], p.center[2] - at[2]) - Math.hypot(q.center[0] - at[0], q.center[1] - at[1], q.center[2] - at[2]); })[0]; };
        var fc = { entry: floorAt(A, 0), wall: cylOf(A, d / 2) };
        if (!last && !through) fc.bottom = floorAt(A, depth);
        if (last) {
          fc.tapWall = cylOf(last, tapD / 2);
          if (!through) { fc.tapEntry = floorAt(last, h.t0); fc.bottom = floorAt(last, h.t0 + depth); }
        }
        if (type === 'counterbore') { fc.cbWall = cylOf(A, cbD / 2); fc.cbFloor = floorAt(A, cbDepth); }
        if (type === 'countersink') fc.csk = sigsOf(A).filter(function (g) { return g.surface === NAMES.coneSurface && g.axis && g.axisOrigin && distToAxis(g) < tol; })[0];
        h.faces = fc;
        h.at = at.map(sig7);
        if (h.t0 !== undefined) h.t0 = sig7(h.t0);
      });
      return { holes: holes, solids: fin, stack: stack, through: through, depth: sig7(depth), normal: n, units: U.unit };
    });
  }, { async: true });

  // N3: 面高亮。args = { faces: [{id, role?: primary|candidate|source}] } —— 每次调用替换上一次; faces 为空 = 清除。
  // 做法: 从面所在实体的显示网格(FaceGroup.mesh + groups)拷出这个面的三角形, 用 gizmo 材质的克隆(改色、透过实体可见)
  // 建一个 Mesh 加到辅助场景(editor.helpers)。不动选择, 鼠标悬停也不会覆盖。
  function clearHighlight() {
    (state.highlights || []).forEach(function (m) {
      try { invokeOn('helpers.remove', [m]); } catch (e) {}
      try { m.geometry.dispose(); m.material.dispose(); } catch (e2) {}
    });
    state.highlights = [];
  }

  function faceMeshCopy(id, style) {
    var m = /^(\d+)[a-z]/.exec(id);
    var sv = m && viewOf('solid', m[1]);
    if (!sv) throw new Error('face not found: ' + id);
    var fg = sv[NAMES.shellGroups.face], src = fg && fg[NAMES.faceMesh];
    var ids = fg ? Array.from(fg[NAMES.groupVersionIds] || []).map(String) : [];
    var fi = ids.indexOf(String(id));
    var grp = fg && fg[NAMES.meshGroups];
    if (!src || fi < 0 || !grp) throw new Error('no display mesh for face ' + id);
    var g = src.geometry, pos = g.attributes.position.array, idx = g.index && g.index.array;
    var start = grp[fi * 2], count = grp[fi * 2 + 1];
    var out = new Float32Array(count * 3);
    for (var i = 0; i < count; i++) {
      var vi = idx ? idx[start + i] : start + i;
      out[i * 3] = pos[vi * 3]; out[i * 3 + 1] = pos[vi * 3 + 1]; out[i * 3 + 2] = pos[vi * 3 + 2];
    }
    var geo = new g.constructor();
    geo.setAttribute('position', new g.attributes.position.constructor(out, 3));
    var mat = need('gizmo.meshMaterial').clone();
    mat.color.set(style.color); mat.opacity = style.opacity; mat.transparent = true;
    mat.depthTest = false; mat.depthWrite = false; mat.side = 2; // DoubleSide
    var mesh = new src.constructor(geo, mat);
    mesh.renderOrder = NAMES.highlightRenderOrder;
    if (src.matrixWorld) { mesh.matrixAutoUpdate = false; mesh.matrix.copy(src.matrixWorld); mesh.matrixWorld.copy(src.matrixWorld); }
    return mesh;
  }

  def('view.highlight', 'view.highlight', function (a) {
    a = a || {};
    clearHighlight();
    var shown = [], failed = [];
    (a.faces || []).forEach(function (f) {
      var id = String(f && f.id !== undefined ? f.id : f);
      var style = NAMES.highlightStyles[(f && f.role) || 'primary'];
      if (!style) { failed.push(id + ': unknown role'); return; }
      try { var mesh = faceMeshCopy(id, style); invokeOn('helpers.add', [mesh]); state.highlights.push(mesh); shown.push(id); }
      catch (e) { failed.push(id + ': ' + errMsg(e)); }
    });
    call0(viewport(), 'vp.render');
    return { shown: shown, failed: failed };
  });

  def('cam.vpRect', 'cam.read', function () {
    var r = need('vp.container').getBoundingClientRect();
    return { x: r.x, y: r.y, width: r.width, height: r.height, scale: 1 };
  });
  // 步进角(弧度)即时生效: Plasticity 不重载 settings.json, 值缓存在各视口轨道控制器上(pitfalls #18)。
  // 内存里的设置也一并改, 防 Plasticity 写回旧值。
  def('cam.setOrbitStep', 'cam.step', function (a) {
    if (!a || typeof a.rad !== 'number') throw new Error('cam.setOrbitStep needs {rad}');
    var settings = put('orbitSettings.step', a.rad);
    var n = 0;
    var vps = get('ed.viewports');
    if (vps) vps.forEach(function (v) { var oc = get('vp.orbit', v); if (oc && put('oc.step', a.rad, oc)) n++; });
    return { viewports: n, settings: settings };
  });

  // ---- 滚转: 轨道控制器目标朝向右乘绕本地 Z 轴的旋转(视线不变、阻尼过渡, 角度 = 步进角) ----
  function viewportOfElement(el) {
    var best = null;
    var vps = get('ed.viewports');
    if (el && vps) {
      vps.forEach(function (v) {
        if (best) return;
        var oc = get('vp.orbit', v);
        var dom = oc && get('oc.domElement', oc);
        if (dom && el.contains && el.contains(dom)) best = v;
      });
    }
    return best || get('ed.activeViewport') || viewport();
  }

  function rollBy(v, sign) {
    var oc = need('vp.orbit', v);
    var cam = need('vp.camera', v);
    var qe = need('oc.quaternionEnd', oc);
    var Q = qe.constructor;
    var V = cam.position.constructor;
    var step = get('oc.step', oc) || Math.PI / 12;
    qe.multiply(new Q().setFromAxisAngle(new V(0, 0, 1), sign * step));
    markOrbitDirty(oc);
    return step;
  }

  def('cam.roll', 'cam.roll', function (a) {
    return { step: rollBy(viewportOfElement(null), a && a.sign < 0 ? -1 : 1) };
  });

  // 在每个视口元素上注册滚转命令(keymap 把 Alt+←/→ 绑到它们)。视口元素重建 / 页面重载后自动补注册。
  // 委托式(见 ensureDelegate): 热替换后执行的是当前实例的 rollBy。
  function maintainRollCommands() {
    var names = CFG.rollCommands;
    var els = document.querySelectorAll(CFG.viewportSelector);
    var n = 0;
    for (var i = 0; i < els.length; i++) {
      if (ensureDelegate(names.left, els[i], 'rollLeft', false)) n++;
      ensureDelegate(names.right, els[i], 'rollRight', false);
    }
    return n;
  }

  // ---- 按键绑定(运行时): 先删同选择器下我们的(现行 / 旧版)绑定, 再按需加 ----
  def('keymap.setBindings', 'keymap', function (a) {
    if (!a || typeof a.bindings !== 'object') throw new Error('keymap.setBindings needs {enable, bindings, legacy?, source?}');
    var km = need('ed.keymaps');
    var list = need('km.bindings');
    var f = NAMES.keyBindingFields;
    var sel = CFG.viewportSelector;
    var cur = a.bindings;
    var old = a.legacy || {};
    var kept = list.filter(function (b) {
      return !(b[f.selector] === sel && (cur[b[f.keys]] === b[f.command] || old[b[f.keys]] === b[f.command]));
    });
    put('km.bindings', kept, km);
    if (a.enable) {
      var m = {};
      m[sel] = cur;
      invokeOn('km.add', [a.source || 'pla-helper', m]);
    }
    try { invokeOn('kb.update'); } catch (e) {}
    return { enabled: !!a.enable, removed: list.length - kept.length };
  });

  // ---- 视口显示状态: 保存 / 恢复(格式与 R8-1 存档兼容) ----
  // 每段独立 try, 失败原因逐段记录(以前空 catch 让整段静默消失, R10)。
  function vecFields(obj) {
    var o = {};
    NAMES.planeVecKeys.forEach(function (k) { o[k] = v3(obj[k]); });
    return o;
  }

  function primFields(src, keys) {
    var o = {};
    keys.forEach(function (k) {
      var v = src[k];
      if (typeof v === 'number' || typeof v === 'boolean' || typeof v === 'string') o[k] = v;
    });
    return o;
  }

  function assignPrims(dst, src, keys) {
    keys.forEach(function (k) {
      if (src[k] === undefined || src[k] === null) return;
      var v = src[k];
      if (typeof dst[k] === 'number' && typeof v === 'string' && v !== '' && !isNaN(Number(v))) v = Number(v);
      try { dst[k] = v; } catch (e) {}
    });
  }

  function listOf(x) {
    if (Array.isArray(x)) return x;
    var a = [];
    try { x.forEach(function (y) { a.push(y); }); } catch (e) {}
    return a;
  }

  // ---- R33: 视口状态 v2 辅助(全部名字走 LOC / NAMES) ----
  function xyz(v) { return v ? { x: v.x, y: v.y, z: v.z } : null; }
  function quat(q) {
    if (!q) return null;
    var pick = function (a, b) { return typeof q[a] === 'number' ? q[a] : q[b]; };
    return { x: pick('x', '_x'), y: pick('y', '_y'), z: pick('z', '_z'), w: pick('w', '_w') };
  }
  function plain(o) {
    return JSON.parse(JSON.stringify(o, function (k, v) { return (v && typeof v === 'object' && ArrayBuffer.isView(v)) ? Array.prototype.slice.call(v) : v; }));
  }
  // 与键序无关的 JSON 比较: 存档经 Rust(serde_json 无 preserve_order)转一圈后对象键按字母排序,
  // 直接比 JSON.stringify 会把"原样"判成"改了"(→ 材质走 memento、文档设置被写成"未保存")。
  function canonJson(o) {
    return JSON.stringify(plain(o), function (k, v) {
      if (!v || typeof v !== 'object' || Array.isArray(v)) return v;
      var s = {};
      Object.keys(v).sort().forEach(function (q) { s[q] = v[q]; });
      return s;
    });
  }
  function sameJson(a, b) { return canonJson(a) === canonJson(b); }
  function texFile(t) { return t && t.userData ? (t.userData[NAMES.matcapFileKey] || null) : null; }
  function baseName(path) { var s = String(path || '').split('/').pop(); return s.replace(/\.[^.]+$/, ''); }

  // 页面上所有带图片的按钮(渲染面板的环境缩略图等): [{el, name}]。
  function imageButtons() {
    var doc = W.document;
    if (!doc || !doc.querySelectorAll) return [];
    var out = [], imgs = doc.querySelectorAll('button img');
    for (var i = 0; i < imgs.length; i++) {
      var b = imgs[i].closest ? imgs[i].closest('button') : null;
      if (b) out.push({ el: b, name: baseName(imgs[i].getAttribute('src')) });
    }
    return out;
  }

  // 当前截面(存档用)。
  function sectionNow() {
    var sh = get('ed.shadingDb');
    var s = get('sh.section', sh);
    var out = { on: !!get('sh.isSectioned', sh) };
    if (s) {
      var bp = s[NAMES.sectionBasePlane];
      out.basePlane = bp ? { n: xyz(bp.normal), c: bp.constant } : null;
      out.basePos = xyz(s[NAMES.sectionBasePos]);
      NAMES.sectionParams.forEach(function (k) { out[k] = s[k]; });
    }
    return out;
  }

  // 恢复截面: 基准平面/位置 → transform(参数) → isSectioned 开关(setter 会同时切换截面模式)。
  function applySection(S) {
    var sh = get('ed.shadingDb');
    if (!S.on) { put('sh.isSectioned', false, sh); return 'section:off'; }
    var s = need('sh.section', sh);
    var bp = s[NAMES.sectionBasePlane];
    if (S.basePlane && bp) {
      if (S.basePlane.n && bp.normal && typeof bp.normal.set === 'function') bp.normal.set(S.basePlane.n.x, S.basePlane.n.y, S.basePlane.n.z);
      if (typeof S.basePlane.c === 'number') bp.constant = S.basePlane.c;
    }
    var pos = s[NAMES.sectionBasePos];
    if (S.basePos && pos && typeof pos.set === 'function') pos.set(S.basePos.x, S.basePos.y, S.basePos.z);
    var args = NAMES.sectionParams.map(function (k) { return S[k]; });
    if (typeof s[NAMES.sectionTransform] === 'function') s[NAMES.sectionTransform].apply(s, args);
    put('sh.isSectioned', true, sh);
    return 'section:on';
  }

  // 当前渲染环境: 文件 + 旋转 + 强度。
  function envNow() {
    var s = invokeOn('sh.snapshot', [], get('ed.shadingDb'));
    return { file: texFile(s.environment), rotation: s.rotation, intensity: s.intensity };
  }

  // 恢复渲染环境: 点同名缩略图(setEnvironment(文件名) 实测会把环境清空, 不用), 再写旋转/强度。
  function applyEnv(S) {
    var sh = get('ed.shadingDb');
    var cur = envNow();
    var parts = [];
    var writeRI = function () {
      if (typeof S.rotation === 'number') put('sh.envRotation', S.rotation, sh);
      if (typeof S.intensity === 'number') put('sh.envIntensity', S.intensity, sh);
    };
    if (S.file && cur.file !== S.file) {
      var want = baseName(S.file), alt = want.replace(NAMES.envResSuffix, '');
      var tryClick = function () {
        var btn = imageButtons().filter(function (b) { return b.name === want || b.name === alt; })[0];
        if (!btn) return false;
        btn.el.click();
        return true;
      };
      if (tryClick()) {
        parts.push('env:' + alt);
      } else if (typeof W.setTimeout === 'function' && W.document && W.document.querySelectorAll) {
        // 刚打开 render mode 时渲染面板(环境缩略图)下一帧才出来 → 每 200ms 再找, 最多 3s
        var tries = 0;
        var again = function () {
          if (tryClick()) { W.setTimeout(writeRI, 800); return; }
          if (++tries < 15) W.setTimeout(again, 200);
        };
        W.setTimeout(again, 200);
        parts.push('env:pending ' + alt);
      } else {
        throw new Error('no env button ' + alt);
      }
    }
    writeRI();
    // 换环境是异步加载, 加载完可能重置旋转/强度 → 稍后再写一次
    if (parts.length && typeof W.setTimeout === 'function') W.setTimeout(writeRI, 800);
    return parts.length ? parts.join(',') : 'env(same)';
  }

  // Shader Mode 面板里带图片的按钮(DOM 由 Plasticity 自己渲染; 找不到 → 空数组, 调用方如实报告)。
  function shaderButtons() {
    var doc = W.document;
    if (!doc || !doc.querySelectorAll) return [];
    var title = null, all = doc.querySelectorAll('*');
    for (var i = 0; i < all.length; i++) {
      var e = all[i];
      if (e.childElementCount === 0 && String(e.textContent || '').trim().toUpperCase() === NAMES.shaderPanelTitle) { title = e; break; }
    }
    var box = title;
    for (var j = 0; j < 6 && box && box.querySelectorAll('button img').length < 8; j++) box = box.parentElement;
    if (!box) return [];
    var out = [];
    var bs = box.querySelectorAll('button');
    for (var k = 0; k < bs.length; k++) { var img = bs[k].querySelector('img'); if (img) out.push({ el: bs[k], name: baseName(img.getAttribute('src')) }); }
    return out;
  }

  // 当前着色(存档用)。button = 应点的面板按钮名(恢复时按它点)。
  function shadingNow() {
    var s = invokeOn('sh.snapshot', [], get('ed.shadingDb'));
    var matcap = texFile(s.matcap);
    var shader = s.shader ? (s.shader.name || null) : null;
    var button = shader ? (NAMES.shaderButtons[shader] || null) : (matcap ? baseName(matcap) : null);
    if (!shader && matcap && baseName(matcap) === NAMES.sharedMatcapFile) {
      // 共用 basic_side 的几个按钮: 以面板激活态为准(面板与内部一致时才采信)
      var act = shaderButtons().filter(function (b) { return b.el.getAttribute(NAMES.shaderButtonStateAttr) === NAMES.shaderButtonActive; })[0];
      button = act && NAMES.sharedMatcapButtons.indexOf(act.name) >= 0 ? act.name : NAMES.sharedMatcapButtons[s.isTinted ? 0 : 1];
    }
    return { mode: s.mode, matcap: matcap, tint: !!s.isTinted, shader: shader, button: button,
             props: plain(s.shaderProperties || {}), rotation: s.rotation, intensity: s.intensity };
  }

  // 恢复着色: 点对应按钮(同面板操作一样完整切换模式), 再把 shader 参数写回。返回结果文字。
  // panelPending = 本次恢复刚关掉 render mode, Shader Mode 面板下一帧才出现 → 找不到按钮时排队重试。
  function applyShading(S, panelPending) {
    var sh = get('ed.shadingDb');
    var cur = shadingNow();
    var sameLook = cur.shader === S.shader && cur.matcap === S.matcap && cur.tint === S.tint;
    var pending = false;
    if (!sameLook) {
      var btn = shaderButtons().filter(function (b) { return b.name === S.button; })[0];
      var docOk = W.document && W.document.querySelectorAll && typeof W.setTimeout === 'function';
      if (btn) {
        btn.el.click();
      } else if (docOk && panelPending) {
        // 刚关掉 render mode 时 Shader Mode 面板下一帧才出来 → 每 200ms 再找, 最多 3s(找到后再写 shader 参数)
        var tries = 0;
        var again = function () {
          var b = shaderButtons().filter(function (x) { return x.name === S.button; })[0];
          if (b) { b.el.click(); W.setTimeout(function () { writeProps(); }, 300); return; }
          if (++tries < 15) W.setTimeout(again, 200);
        };
        W.setTimeout(again, 200);
        pending = true;
      } else if (!S.shader && S.matcap && !cur.shader && LOC['sh.setMatcap'] && get('sh.setMatcap')) {
        // 兜底(自定义 matcap 等面板上没有的): 只在当前不是 shader 模式时直接 setMatcap
        invokeOn('sh.setMatcap', [S.matcap, S.tint], sh);
      } else {
        throw new Error('no button ' + S.button);
      }
    }
    var props = S.props || {};
    var writeProps = function () {
      var s = invokeOn('sh.snapshot', [], sh);
      if ((s.shader ? s.shader.name : null) !== S.shader) return false;
      var merged = {};
      Object.keys(s.shaderProperties || {}).forEach(function (k) { merged[k] = s.shaderProperties[k]; });
      Object.keys(props).forEach(function (k) { merged[k] = props[k]; });
      s.shaderProperties = merged;
      invokeOn('sh.init', [s], sh);
      return true;
    };
    if (S.shader && Object.keys(props).length) {
      // 按钮切换若是异步的, 稍后再写一次参数
      if (!writeProps() && typeof W.setTimeout === 'function') W.setTimeout(writeProps, 400);
    }
    return sameLook ? 'shading(same)' : (pending ? 'shading:pending ' : 'shading:') + S.button;
  }

  // memento 恢复(originator.restore)前的前置条件。实测(2026-11-14 夜): 窗口不可见(visibilityState=hidden,
  // 显示器关闭/被遮挡)时连"原样恢复当前快照"都在 Plasticity 内部抛 "reading 'Viewport'" —— 不冒这个险。
  // 进行中的命令先取消(同 sel.apply; 26.2.3 上 hasActiveCommand 是常为 true 的属性 —— 默认选择命令一直在,
  // 不能拿它当"拒绝"条件)。
  function mementoGuard() {
    if (W.document && W.document.visibilityState === 'hidden') throw new Error('window hidden');
    var exec = get('ed.executor');
    if (exec && get('exec.cancel')) { try { invokeOn('exec.cancel', [], exec); } catch (e) {} }
  }

  // 隐藏 / 眼睛关闭 → 稳定 id 列表。
  function visibilityNow() {
    var st = need('nodes.state');
    var map = st[NAMES.node2id];
    var ids = function (set) { var a = []; set.forEach(function (k) { var id = map.get(k); if (id) a.push(id); }); return a; };
    var out = {};
    NAMES.nodeVisSets.forEach(function (n) { out[n] = ids(st[n]); });
    // 隔离 = 层栈(外 → 内), 每层是 nodeKey 的 Set(实测 26.2.3); 存成每层一个稳定 id 列表
    out[NAMES.nodeIsolations] = (st[NAMES.nodeIsolations] || []).map(function (layer) { return ids(toSet(layer)); });
    return out;
  }

  // Set / 数组 / 类型化数组 → Set(隔离层的几种可能形态)。
  function toSet(x) {
    if (x instanceof Set) return x;
    var s = new Set();
    if (x && typeof x.forEach === 'function') x.forEach(function (k) { s.add(k); });
    else if (x && typeof x.length === 'number') for (var i = 0; i < x.length; i++) s.add(x[i]);
    return s;
  }

  // 恢复可见性: 只改 memento 的 nodes 可见性字段, 走编辑器自己的 restore(与撤销同一条路, 场景才会刷新;
  // 直接 setHidden 只改集合不刷新)。其余部分取当下快照 = 原地不动。
  function applyVisibility(V, missing) {
    var st = need('nodes.state');
    var inv = new Map();
    st[NAMES.node2id].forEach(function (id, k) { inv.set(id, k); });
    var keysOf = function (list) {
      var ks = [];
      (list || []).forEach(function (id) { var k = inv.get(id); if (k === undefined) missing.push('node:' + id); else ks.push(k); });
      return ks;
    };
    var want = {}, visSame = true;
    var sameSet = function (cur, keys) { return cur.size === keys.length && keys.every(function (k) { return cur.has(k); }); };
    NAMES.nodeVisSets.forEach(function (n) { want[n] = keysOf(V[n]); });
    // 隔离(2026-11-15 用户实测: 直接写 NodeState 的隔离层后 Plasticity 卡在隔离里, Alt+. 退不出来 —— 它自己的 IsolateCommand
    // 还有 preisolate 等步骤): 不再直接写隔离层。存档里的隔离 → 换成"隐藏隔离集合之外的物体"(普通隐藏, Plasticity 照常处理,
    // 全部显示即可还原); 当前处于隔离 → 派发 Plasticity 自己的退出隔离命令(= 按 Alt+.)。
    var isoSaved = Array.isArray(V[NAMES.nodeIsolations]) && V[NAMES.nodeIsolations].every(Array.isArray) ? V[NAMES.nodeIsolations] : null;
    var isoTop = isoSaved ? isoSaved.map(keysOf).filter(function (l) { return l.length > 0; }).pop() : null;
    if (isoTop) {
      var keep = new Set(isoTop), hid = new Set(want[NAMES.nodeVisSets[0]]);
      Array.from(need('geo.items') || []).forEach(function (it) {
        var v = it && it[NAMES.lookupViewKey], k;
        try { k = invokeOn('nodes.item2key', [v]); } catch (e) { return; }
        if (k !== null && k !== undefined && !keep.has(k)) hid.add(k);
      });
      want[NAMES.nodeVisSets[0]] = Array.from(hid);
    }
    NAMES.nodeVisSets.forEach(function (n) { if (!sameSet(st[n], want[n])) visSame = false; });
    var curIso = (st[NAMES.nodeIsolations] || []).length;
    if (visSame && !curIso) return 'vis(same)';
    var parts = [];
    if (!visSame) {
      mementoGuard();
      var orig = need('ed.originator');
      // 隐藏 / 眼睛: 改 memento 的两项再 restore(场景随之重算)
      var m = invokeOn('orig.snapshot', [], orig);
      var nodes = {};
      Object.keys(m[NAMES.mementoNodes]).forEach(function (k) { nodes[k] = m[NAMES.mementoNodes][k]; });
      NAMES.nodeVisSets.forEach(function (n) { nodes[n] = Float64Array.from(want[n]); });
      m[NAMES.mementoNodes] = nodes;
      invokeOn('orig.restore', [m], orig);
      parts.push(want[NAMES.nodeVisSets[0]].length + '/' + want[NAMES.nodeVisSets[1]].length + (isoTop ? ' iso→hidden' : ''));
    }
    // 退出隔离放最后: 恢复前的 mementoGuard 会取消活动命令, 先派发的话会把刚排上的退出隔离命令取消掉
    if (curIso) {
      for (var i = 0; i < curIso; i++) W.document.body.dispatchEvent(new CustomEvent(NAMES.unisolateCommand, { bubbles: true }));
      parts.push('unisolate' + curIso);
    }
    return 'vis:' + parts.join('/');
  }

  def('vp.save', 'vp.state', function () {
    var out = { failed: [] };
    function seg(name, fn) { try { fn(); } catch (e) { out.failed.push(name + ':' + errMsg(e)); } }
    seg('cam', function () {
      var cam = need('vp.camera');
      var c = { px: cam.position.x, py: cam.position.y, pz: cam.position.z,
                qx: cam.quaternion.x, qy: cam.quaternion.y, qz: cam.quaternion.z, qw: cam.quaternion.w };
      NAMES.cameraExtraKeys.forEach(function (k) { c[k] = typeof cam[k] === 'number' ? cam[k] : null; });
      out.cam = c;
    });
    seg('orbit', function () {
      var oc = need('vp.orbit');
      var o = {};
      ORBIT_FIELDS.forEach(function (f) {
        [f[0], f[0] + 'End'].forEach(function (name) {
          var val = get('oc.' + name, oc);
          o[name] = f[3] === 'v3' ? v3(val) : (typeof val === 'number' ? val : null);
        });
      });
      out.orbit = o;
    });
    seg('shading', function () {
      var tag = NAMES.shadingTagKey;
      var m = get('ed.shading.mode');
      var s = get('ed.shading.section');
      out.shading = { mode: m ? m[tag] : null, section: s ? s[tag] : null };
    });
    seg('sections', function () {
      out.sections = listOf(need('ed.sections')).map(function (s) {
        var o = vecFields(s);
        o.enabled = !!s.enabled;
        return o;
      });
    });
    seg('cplane', function () { var cp = get('vp.cplane'); if (cp) out.cplane = vecFields(cp); });
    // 旧版(26.2.3 之前)的视口设置路径; 26.2.3 上不存在 → 不记失败(网格/色调/AO 改由文档设置 doc 段保存)
    if (get('vp.settings.renderer')) seg('renderer', function () { out.renderer = primFields(need('vp.settings.renderer'), NAMES.rendererKeys); });
    if (get('vp.settings.ao')) seg('ao', function () {
      var ao = need('vp.settings.ao');
      out.ao = primFields(ao, Object.keys(ao).filter(function (k) { return typeof ao[k] !== 'string'; }));
    });
    if (get('vp.settings.grid')) seg('grid', function () { out.grid = primFields(need('vp.settings.grid'), NAMES.gridKeys); });
    // ---- R33 v2: 显示相关全部状态(旧字段保留, 老存档照常恢复) ----
    out.v = NAMES.vpStateVersion;
    seg('view', function () {
      var V = viewport();
      var s = invokeOn('vp.snapshot', [], V);
      var oc = get('vp.orbit');
      var zoom = oc && typeof get('oc.zoomEnd', oc) === 'number' ? get('oc.zoomEnd', oc) : s.camera.zoom;
      out.view = {
        camera: { mode: s.camera.mode, position: xyz(s.camera.position), quaternion: quat(s.camera.quaternion), zoom: zoom, fov: s.camera.fov },
        target: xyz(s.target), focalOffset: xyz(s.focalOffset), isXRay: !!s.isXRay, showOverlays: !!s.showOverlays,
        constructionPlane: s.constructionPlane ? { n: xyz(s.constructionPlane.n), o: xyz(s.constructionPlane.o) } : null
      };
    });
    seg('renderMode', function () { out.renderMode = !!need('vp.isRenderMode'); });
    seg('scalars', function () { var V = viewport(); out.scalars = primFields(V, NAMES.vpScalars); });
    seg('layers', function () { out.layers = primFields(need('ed.layers'), NAMES.layerFlags); });
    seg('shading2', function () { out.shading2 = shadingNow(); });
    seg('section2', function () { out.section2 = sectionNow(); });
    seg('env', function () { out.env = envNow(); });
    seg('doc', function () { out.doc = plain(invokeOn('ds.snapshot', [], need('ed.docSettings'))); });
    seg('vis', function () { out.vis = visibilityNow(); });
    try {
      var r = need('vp.container').getBoundingClientRect();
      out.vpRect = [r.x, r.y, r.width, r.height];
      out.dpr = W.devicePixelRatio || 1;
    } catch (e) {}
    out.err = '';
    return out;
  });

  def('vp.restore', 'vp.state', function (S) {
    S = S || {};
    var applied = [], missing = [], failed = [];
    function seg(name, present, fn) {
      if (!present) { missing.push(name); return; }
      try { fn(); } catch (e) { failed.push(name + ':' + errMsg(e)); }
    }
    var v2 = S.v === NAMES.vpStateVersion;
    var renderToggled = false;
    // ---- R33 v2 前段: 可见性 → 文档设置(网格/色调/AO) → render mode(它会改着色, 必须在着色之前) ----
    if (v2) {
      seg('vis', !!S.vis, function () { applied.push(applyVisibility(S.vis, missing)); });
      seg('doc', !!S.doc, function () {
        var ds = need('ed.docSettings');
        // 文档设置一写就会让文件变"未保存" → 没变化就不写
        if (sameJson(invokeOn('ds.snapshot', [], ds), S.doc)) { applied.push('doc(same)'); return; }
        invokeOn('ds.init', [S.doc], ds);
        var sig = get('ed.sig.docSettings');
        if (sig && typeof sig.dispatch === 'function') sig.dispatch();
        applied.push('doc');
      });
      seg('renderMode', typeof S.renderMode === 'boolean', function () {
        var V = viewport();
        if (!!need('vp.isRenderMode') !== S.renderMode) { invokeOn('vp.toggleRenderMode', [], V); renderToggled = true; }
        applied.push('renderMode');
      });
    }
    seg('cam', !!S.cam, function () {
      var cam = need('vp.camera');
      var view = {};
      Object.keys(S.cam).forEach(function (k) { view[k] = S.cam[k]; });
      if (S.orbit) view.orbit = S.orbit;
      // 正交相机不写投影参数(R7-31h) — 如实标注没应用的项。
      if (cam[NAMES.perspectiveFlag] !== true) {
        if (typeof S.cam.zoom === 'number') missing.push('zoom(正交未应用)');
        if (typeof S.cam.fov === 'number') missing.push('fov(正交未应用)');
      }
      var r = API['cam.set'].fn(view);
      applied.push('cam');
      if (r.proj) applied.push('proj');
      if (r.orbit) applied.push('orbit');
    });
    if (!S.orbit) missing.push('orbit');
    // v2 存档用 shading2(点按钮); 旧的改 tag 链只给老存档用。
    seg('shading', !!(S.shading && S.shading.mode) && !(v2 && S.shading2), function () {
      var tag = NAMES.shadingTagKey;
      var m = need('ed.shading.mode');
      if (m[tag] !== S.shading.mode) {
        // 切换链(R8-1 实测): 构造合法 prev → 改 tag → 派发 shadingChanged。
        var prev = {};
        prev[tag] = m[tag];
        NAMES.shadingPrevKeys.forEach(function (k) { prev[k] = m[k]; });
        m[NAMES.shadingPrevField] = prev;
        m[tag] = S.shading.mode;
        need('ed.shading.changed').dispatch(m);
      }
      if (S.shading.section) { var sm = get('ed.shading.section'); if (sm) sm[tag] = S.shading.section; }
      applied.push('shading');
    });
    // v2 存档的视口截面在 section2(shading.section); 旧的 sections 段(SectionDatabase)只给老存档用
    seg('sections', Array.isArray(S.sections) && !(v2 && S.section2), function () {
      var cur = listOf(need('ed.sections'));
      var n = Math.min(cur.length, S.sections.length);
      for (var i = 0; i < n; i++) {
        var d = S.sections[i], s = cur[i];
        try { s.enabled = !!d.enabled; } catch (e) {}
        NAMES.planeVecKeys.forEach(function (k) { setVec(s[k], d[k]); });
      }
      // 数量对齐: 多余的禁用; 缺的无法凭空造(如实只恢复 n 个)。
      for (var j = S.sections.length; j < cur.length; j++) { try { cur[j].enabled = false; } catch (e2) {} }
      applied.push('sections:' + n);
    });
    seg('cplane', !!S.cplane, function () {
      var cp = get('vp.cplane');
      if (!cp) throw new Error('null');
      NAMES.planeVecKeys.forEach(function (k) { setVec(cp[k], S.cplane[k]); });
      applied.push('cplane');
    });
    seg('renderer', !!S.renderer, function () { assignPrims(need('vp.settings.renderer'), S.renderer, NAMES.rendererKeys); applied.push('renderer'); });
    seg('ao', !!S.ao, function () { var ao = need('vp.settings.ao'); assignPrims(ao, S.ao, Object.keys(S.ao)); applied.push('ao'); });
    seg('grid', !!S.grid, function () { assignPrims(need('vp.settings.grid'), S.grid, NAMES.gridKeys); applied.push('grid'); });
    // ---- R33 v2 后段: 着色(render mode 下由它接管, 不动) → 视角(vp.init) → 缩放 → 标量 → 显示开关 ----
    if (v2) {
      seg('shading2', !!S.shading2 && !S.renderMode, function () { applied.push(applyShading(S.shading2, renderToggled)); });
      // render mode 下的环境(文件/旋转/强度); render mode 关时不动
      if (S.renderMode === true) seg('env', !!S.env, function () { applied.push(applyEnv(S.env)); });
      seg('section2', !!S.section2, function () { applied.push(applySection(S.section2)); });
      seg('view', !!S.view, function () {
        var V = viewport();
        var cur = invokeOn('vp.snapshot', [], V);
        // 透视/正交: vp.init 不切换投影(实测), 模式不同先用视口自己的切换
        var wantMode = S.view.camera && S.view.camera.mode;
        if (wantMode && cur.camera && cur.camera.mode !== wantMode && get('vp.togglePerspective')) {
          invokeOn('vp.togglePerspective', [], V);
          cur = invokeOn('vp.snapshot', [], V);
          applied.push('projection');
        }
        var next = {};
        Object.keys(cur).forEach(function (k) { next[k] = cur[k]; });
        var cam = {};
        Object.keys(cur.camera).forEach(function (k) { cam[k] = cur.camera[k]; });
        Object.keys(S.view.camera || {}).forEach(function (k) { cam[k] = S.view.camera[k]; });
        next.camera = cam;
        ['target', 'focalOffset', 'isXRay', 'showOverlays'].forEach(function (k) { if (S.view[k] !== undefined && S.view[k] !== null) next[k] = S.view[k]; });
        if (S.view.constructionPlane) next.constructionPlane = S.view.constructionPlane;
        invokeOn('vp.init', [next], V);
        // vp.init 不恢复缩放: 写轨道控制器的 zoom / zoomEnd(阻尼两份)
        var oc = get('vp.orbit');
        var z = S.view.camera && S.view.camera.zoom;
        if (oc && typeof z === 'number') { put('oc.zoom', z, oc); put('oc.zoomEnd', z, oc); }
        applied.push('view');
      });
      seg('scalars', !!S.scalars, function () { assignPrims(viewport(), S.scalars, NAMES.vpScalars); applied.push('scalars'); });
      seg('layers', !!S.layers, function () { assignPrims(need('ed.layers'), S.layers, NAMES.layerFlags); applied.push('layers'); });
    }
    call0(viewport(), 'vp.render');
    if (v2) {
      // v2 存档里旧格式段落本来就没有(或已由新段落取代), 不算"缺失"; render mode 下跳过着色也是预期
      var legacy = ['cam', 'orbit', 'shading', 'sections', 'cplane', 'renderer', 'ao', 'grid', 'shading2'];
      missing = missing.filter(function (m) { return legacy.indexOf(m) < 0; });
    }
    return { applied: applied, missing: missing, failed: failed };
  });

  // ---- R34: 材质与颜色状态 —— 材质定义(glTF 风格: baseColorFactor / metallic / roughness …, uuid 稳定)
  //      + 物体 → 材质的指派(物体按稳定 id, 材质按 uuid)。恢复走 memento(同可见性), 不进撤销栈。 ----
  function materialsNow() {
    // 材质定义优先读材质库自己的 snapshot(与 memento 的 materials 段同形): 整份 memento 在有命令进行中时
    // (如用户开着材质编辑 edit-material, 测量库处于 scratch)会抛 "snapshot(): scratch active"(用户 2026-11-15 实测)。
    var defs = null;
    try { if (get('mats.snapshot')) defs = invokeOn('mats.snapshot', [])[NAMES.materialsList]; } catch (e) { defs = null; }
    if (!Array.isArray(defs)) {
      var m = invokeOn('orig.snapshot', [], need('ed.originator'));
      defs = m[NAMES.mementoMaterials][NAMES.materialsList] || [];
    }
    defs = plain(defs);
    var id2uuid = {};
    defs.forEach(function (d) { id2uuid[d[NAMES.materialId]] = d[NAMES.materialUuid]; });
    var st = need('nodes.state');
    var assign = [];
    st[NAMES.node2material].forEach(function (mid, k) {
      var sid = st[NAMES.node2id].get(k);
      if (sid && id2uuid[mid] !== undefined) assign.push([sid, id2uuid[mid]]);
    });
    assign.sort(function (a, b) { return a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0; });
    return { v: 1, materials: defs, assign: assign };
  }

  def('mat.save', 'mat.state', function () { return materialsNow(); });

  // R34d: 存档里的贴图是 Texture.toJSON 的纯数据, Plasticity 恢复时(deserializeTexture)要真 Texture 对象,
  // 否则抛 "undefined is not a function"(实测: 带贴图的 Gold foil)。同 uuid 同文件的贴图直接沿用当前对象;
  // 否则按文件名经材质库的 TextureLoader 加载再套回存档里的参数; 没有文件名的(导入图片)如实列入 failed 并去掉该槽。
  function isTexSlot(v) {
    var t = v && typeof v === 'object' && !Array.isArray(v) ? v[NAMES.materialTexSlot] : null;
    return !!(t && typeof t === 'object');
  }
  function reviveTextures(defs, failed) {
    var live = {};
    var m = invokeOn('orig.snapshot', [], need('ed.originator'));
    (m[NAMES.mementoMaterials][NAMES.materialsList] || []).forEach(function (d) { live[d[NAMES.materialUuid]] = d; });
    var jobs = [];
    var out = defs.map(function (d0) {
      var d = {};
      Object.keys(d0).forEach(function (k) { d[k] = d0[k]; });
      Object.keys(d).forEach(function (k) {
        if (!isTexSlot(d[k])) return;
        var j = d[k][NAMES.materialTexSlot];
        if (typeof j.clone === 'function') return; // 已是真对象
        var file = texFile(j);
        var l = live[d[NAMES.materialUuid]];
        var slot = {};
        Object.keys(d[k]).forEach(function (s) { slot[s] = d[k][s]; });
        d[k] = slot;
        if (l && isTexSlot(l[k]) && file && texFile(l[k][NAMES.materialTexSlot]) === file) { slot[NAMES.materialTexSlot] = l[k][NAMES.materialTexSlot]; return; }
        if (!file || !get('tex.get')) { failed.push('texture:' + d.name + '.' + k); delete d[k]; return; }
        jobs.push(Promise.resolve(invokeOn('tex.get', [file], get('mat.textures'))).then(function (T) {
          var t = T.clone();
          t.userData = Object.assign({}, j.userData);
          if (Array.isArray(j.repeat)) t.repeat.fromArray(j.repeat);
          if (Array.isArray(j.offset)) t.offset.fromArray(j.offset);
          if (Array.isArray(j.center)) t.center.fromArray(j.center);
          if (typeof j.rotation === 'number') t.rotation = j.rotation;
          if (Array.isArray(j.wrap)) { t.wrapS = j.wrap[0]; t.wrapT = j.wrap[1]; }
          if (typeof j.flipY === 'boolean') t.flipY = j.flipY;
          if (j.colorSpace) t.colorSpace = j.colorSpace;
          t.needsUpdate = true;
          slot[NAMES.materialTexSlot] = t;
        }, function (e) { failed.push('texture:' + d.name + '.' + k + ':' + errMsg(e)); delete d[k]; }));
      });
      return d;
    });
    return Promise.all(jobs).then(function () { return out; });
  }

  def('mat.restore', 'mat.state', function (S) {
    if (!S || !Array.isArray(S.materials) || !Array.isArray(S.assign)) throw new Error('mat.restore needs {materials, assign}');
    var cur = materialsNow();
    if (sameJson(cur.materials, S.materials) && sameJson(cur.assign, S.assign)) {
      return { applied: ['materials(same)'], missing: [], failed: [] };
    }
    mementoGuard();
    var failed = [];
    return reviveTextures(S.materials, failed).then(function (defs) { return restoreMaterials(S, defs, failed); });
  }, { async: true });

  function restoreMaterials(S, defs, failed) {
    mementoGuard(); // 加载贴图期间窗口可能被隐藏
    var missing = [];
    var uuid2id = {}, maxId = 0;
    S.materials.forEach(function (d) { uuid2id[d[NAMES.materialUuid]] = d[NAMES.materialId]; if (d[NAMES.materialId] > maxId) maxId = d[NAMES.materialId]; });
    var st = need('nodes.state');
    var inv = new Map();
    st[NAMES.node2id].forEach(function (id, k) { inv.set(id, k); });
    var keys = [], vals = [];
    S.assign.forEach(function (p) {
      var k = inv.get(p[0]), id = uuid2id[p[1]];
      if (k === undefined) missing.push('node:' + p[0]);
      else if (id === undefined) missing.push('material:' + p[1]);
      else { keys.push(k); vals.push(id); }
    });
    var orig = need('ed.originator');
    var m = invokeOn('orig.snapshot', [], orig);
    var mats = {};
    Object.keys(m[NAMES.mementoMaterials]).forEach(function (k) { mats[k] = m[NAMES.mementoMaterials][k]; });
    mats[NAMES.materialsList] = defs;
    mats[NAMES.materialsCounter] = Math.max(mats[NAMES.materialsCounter] || 0, maxId + 1);
    m[NAMES.mementoMaterials] = mats;
    var nodes = {};
    Object.keys(m[NAMES.mementoNodes]).forEach(function (k) { nodes[k] = m[NAMES.mementoNodes][k]; });
    var pair = nodes[NAMES.mementoNodeMaterials];
    // 沿用 memento 原来的类型化数组类型
    nodes[NAMES.mementoNodeMaterials] = [new pair[0].constructor(keys), new pair[1].constructor(vals)];
    m[NAMES.mementoNodes] = nodes;
    // 实测(R34d): restore 中途抛错时指派已写入、材质没写入 → 物体指向不存在的材质。失败就用恢复前的快照整体退回。
    var before = invokeOn('orig.snapshot', [], orig);
    try {
      invokeOn('orig.restore', [m], orig);
    } catch (e) {
      var rb;
      try { invokeOn('orig.restore', [before], orig); rb = 'rolled back'; } catch (e2) { rb = 'rollback failed: ' + errMsg(e2); }
      throw new Error(errMsg(e) + ' (' + rb + ')');
    }
    // 实测(R34, 26.2.3): memento 恢复后数据已变, 但视口不会自己重画, 要等下一次交互才看到新颜色 → 主动请求一帧
    call0(viewport(), 'vp.render');
    return { applied: ['materials:' + defs.length, 'assign:' + keys.length], missing: missing, failed: failed };
  }

  // ---- C0 配置(用户 2026-11-15): 固化一个方案 = 可见性(稳定 id)+ 材质指派; 不含相机 / 着色等纯视觉要素。
  //      抓取 / 恢复复用视口状态的 visibilityNow / applyVisibility 与材质状态的 materialsNow / restoreMaterials。
  //      members = 此刻可见的实体(稳定 id + 名字), BOM 按配置计算时直接用, 不必切换显示。 ----
  function visibleSolids() {
    var st = need('nodes.state');
    var iso = st[NAMES.nodeIsolations] || [];
    var top = iso.length ? toSet(iso[iso.length - 1]) : null;
    var hid = st[NAMES.nodeVisSets[0]], inv = st[NAMES.nodeVisSets[1]];
    var out = [];
    // 父组被隐藏的实体也不算可见(大纲里关掉整个"方案"组时, 组里实体自己的标记还是可见)
    var G = get('ed.groupsDb'), groupHidden = new Map();
    function underHiddenGroup(k) {
      if (!G) return false;
      var chain = [], hidden = false;
      for (var guard = 0; guard < 32; guard++) {
        var g;
        try { g = invokeOn('groups.getParent', [k], G); } catch (e) { break; }
        if (!g || g[NAMES.bom.groupId] === NAMES.bom.rootGroupId) break;
        var gk = invokeOn('nodes.item2key', [g]);
        if (groupHidden.has(gk)) { hidden = groupHidden.get(gk); break; }
        chain.push(gk);
        if (hid.has(gk) || inv.has(gk)) { hidden = true; break; }
        k = gk;
      }
      chain.forEach(function (c) { groupHidden.set(c, hidden); });
      return hidden;
    }
    solidViews().forEach(function (v) {
      var k = invokeOn('nodes.item2key', [v]);
      if (hid.has(k) || inv.has(k) || (top && !top.has(k)) || underHiddenGroup(k)) return;
      var sid = st[NAMES.node2id].get(k);
      if (sid) out.push({ id: sid, name: nameOf(v) });
    });
    out.sort(function (a, b) { return a.name < b.name ? -1 : a.name > b.name ? 1 : 0; });
    return out;
  }

  // → { v, vis, mat, members: [{id, name}] }
  def('cfg.save', 'cfg.state', function () {
    return { v: 1, vis: visibilityNow(), mat: materialsNow(), members: visibleSolids() };
  });

  // args = 配置(cfg.save 的回包), parts?: { vis: bool, mat: bool }(默认两项都恢复) → { applied, missing, failed }
  def('cfg.restore', 'cfg.state', function (S) {
    S = S || {};
    var parts = S.parts || {};
    var applied = [], missing = [], failed = [];
    if (S.vis && parts.vis !== false) {
      try { applied.push(applyVisibility(S.vis, missing)); } catch (e) { failed.push('vis:' + errMsg(e)); }
    }
    if (!(S.mat && parts.mat !== false) || !Array.isArray(S.mat.materials) || !Array.isArray(S.mat.assign)) {
      return Promise.resolve({ applied: applied, missing: missing, failed: failed });
    }
    var cur = materialsNow();
    if (sameJson(cur.materials, S.mat.materials) && sameJson(cur.assign, S.mat.assign)) {
      applied.push('materials(same)');
      return Promise.resolve({ applied: applied, missing: missing, failed: failed });
    }
    try { mementoGuard(); } catch (e) { failed.push('mat:' + errMsg(e)); return Promise.resolve({ applied: applied, missing: missing, failed: failed }); }
    var tf = [];
    return reviveTextures(S.mat.materials, tf).then(function (defs) {
      var r = restoreMaterials(S.mat, defs, tf);
      return { applied: applied.concat(r.applied), missing: missing.concat(r.missing), failed: failed.concat(r.failed) };
    }, function (e) { failed.push('mat:' + errMsg(e)); return { applied: applied, missing: missing, failed: failed }; });
  }, { async: true });

  // ---- B1/B2 BOM(PRD-bom-drawings §2): 范围 = 配置的 members(或当前可见实体); 相同零件 = 几何指纹相同(与位置 / 朝向无关);
  //      镜像件(手性相反)单列; 组 = 层级路径。指纹全部由显示网格算(同 geo.measure mass)。 ----
  // 3×3 对称阵特征分解(Jacobi): → { vals, vecs(列向量) }
  function eig3(A) {
    var a = [[A[0][0], A[0][1], A[0][2]], [A[1][0], A[1][1], A[1][2]], [A[2][0], A[2][1], A[2][2]]];
    var v = [[1, 0, 0], [0, 1, 0], [0, 0, 1]];
    for (var sweep = 0; sweep < 50; sweep++) {
      var off = Math.abs(a[0][1]) + Math.abs(a[0][2]) + Math.abs(a[1][2]);
      if (off < 1e-30) break;
      for (var p = 0; p < 2; p++) for (var q = p + 1; q < 3; q++) {
        if (Math.abs(a[p][q]) < 1e-300) continue;
        var th = (a[q][q] - a[p][p]) / (2 * a[p][q]);
        var t = (th >= 0 ? 1 : -1) / (Math.abs(th) + Math.sqrt(th * th + 1));
        var c = 1 / Math.sqrt(t * t + 1), s = t * c;
        for (var k = 0; k < 3; k++) { var akp = a[k][p], akq = a[k][q]; a[k][p] = c * akp - s * akq; a[k][q] = s * akp + c * akq; }
        for (var k2 = 0; k2 < 3; k2++) { var apk = a[p][k2], aqk = a[q][k2]; a[p][k2] = c * apk - s * aqk; a[q][k2] = s * apk + c * aqk; }
        for (var k3 = 0; k3 < 3; k3++) { var vkp = v[k3][p], vkq = v[k3][q]; v[k3][p] = c * vkp - s * vkq; v[k3][q] = s * vkp + c * vkq; }
      }
    }
    return { vals: [a[0][0], a[1][1], a[2][2]], vecs: [[v[0][0], v[1][0], v[2][0]], [v[0][1], v[1][1], v[2][1]], [v[0][2], v[1][2], v[2][2]]] };
  }

  // 一个实体的几何指纹(文档单位): 体积、面积、各曲面类型面数、主二阶矩(√(λ/V), 长度量纲, 升序)、手性(+1 / −1 / 0 = 对称或分不清)
  function partPrint(view, k) {
    var fg = view[NAMES.shellGroups.face], mesh = fg && fg[NAMES.faceMesh], g = mesh && mesh.geometry;
    var pos = g && g.attributes && g.attributes.position && g.attributes.position.array, idx = g && g.index && g.index.array;
    if (!pos || !idx) throw new Error('no display mesh for solid ' + itemIdOf(view));
    var me = mesh.matrixWorld && mesh.matrixWorld.elements;
    var P = function (i) { var j = idx[i] * 3; return xfPoint(me, pos[j], pos[j + 1], pos[j + 2]); };
    var o = P(0);   // 以网格上一点为原点, 减小 Float32 / 大坐标的误差
    var vol = 0, area = 0, m = [0, 0, 0], C = [[0, 0, 0], [0, 0, 0], [0, 0, 0]], tris = [];
    for (var i = 0; i + 2 < idx.length; i += 3) {
      var a = P(i), b = P(i + 1), c = P(i + 2);
      a = [a[0] - o[0], a[1] - o[1], a[2] - o[2]]; b = [b[0] - o[0], b[1] - o[1], b[2] - o[2]]; c = [c[0] - o[0], c[1] - o[1], c[2] - o[2]];
      var det = a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0]);
      vol += det / 6;
      var sx = a[0] + b[0] + c[0], sy = a[1] + b[1] + c[1], sz = a[2] + b[2] + c[2];
      m[0] += det / 6 * sx / 4; m[1] += det / 6 * sy / 4; m[2] += det / 6 * sz / 4;
      var S = [sx, sy, sz], pts = [a, b, c];
      for (var r = 0; r < 3; r++) for (var q = 0; q < 3; q++) {
        C[r][q] += det / 120 * (pts[0][r] * pts[0][q] + pts[1][r] * pts[1][q] + pts[2][r] * pts[2][q] + S[r] * S[q]);
      }
      var ux = b[0] - a[0], uy = b[1] - a[1], uz = b[2] - a[2], vx = c[0] - a[0], vy = c[1] - a[1], vz = c[2] - a[2];
      var ta = Math.hypot(uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx) / 2;
      area += ta;
      tris.push([sx / 3, sy / 3, sz / 3, ta]);
    }
    if (vol < 0) { vol = -vol; m = m.map(function (x) { return -x; }); C = C.map(function (row) { return row.map(function (x) { return -x; }); }); }
    var cen = vol > 0 ? [m[0] / vol, m[1] / vol, m[2] / vol] : [0, 0, 0];
    // 平移到质心(平行轴)
    for (var r2 = 0; r2 < 3; r2++) for (var q2 = 0; q2 < 3; q2++) C[r2][q2] -= vol * cen[r2] * cen[q2];
    var E = eig3(C);
    var order = [0, 1, 2].sort(function (x, y) { return E.vals[x] - E.vals[y]; });
    var e1 = E.vecs[order[0]], e2 = E.vecs[order[1]], e3 = vcross(e1, e2);   // 右手系
    var moments = order.map(function (j) { return vol > 0 ? Math.sqrt(Math.max(0, E.vals[j]) / vol) / k : 0; });
    // 手性: 主轴系里表面三阶矩之积的符号(翻任一轴的方向, 为保持右手系另一轴也翻, 积不变; 镜像 → 积变号)
    var s = [0, 0, 0];
    tris.forEach(function (t) {
      var d = [t[0] - cen[0], t[1] - cen[1], t[2] - cen[2]];
      var p1 = vdot(d, e1), p2 = vdot(d, e2), p3 = vdot(d, e3);
      s[0] += t[3] * p1 * p1 * p1; s[1] += t[3] * p2 * p2 * p2; s[2] += t[3] * p3 * p3 * p3;
    });
    var L = Math.max(moments[2] * k, 1e-12), eps = NAMES.bom.chiralEps * area * L * L * L;
    var chiral = Math.abs(s[0]) > eps && Math.abs(s[1]) > eps && Math.abs(s[2]) > eps ? (s[0] * s[1] * s[2] > 0 ? 1 : -1) : 0;
    // 各曲面类型面数
    var counts = {};
    groupEntries(view, 'face', 1).forEach(function (e) { var md = modelOf('face', e.id); var t; try { t = md[NAMES.surfaceGetter]()[NAMES.surfaceInnerKey].constructor.name; } catch (x) { t = '?'; } counts[t] = (counts[t] || 0) + 1; });
    var s5 = function (x) { return Number(Number(x).toPrecision(5)); };
    return { volume: s5(vol / (k * k * k)), area: s5(area / (k * k)), moments: moments.map(s5), faces: counts, chiral: chiral };
  }

  // 标准件识别用的回转面摘要(文档单位; 面数 > shapeMaxFaces → null)。分类在 Rust(std_parts.rs)。
  // → { faces, planes, len(沿主轴的总长, 由显示网格顶点投影), rev: [{ t: 'cyl'|'cone', r, h(圆柱轴向长), tc(面中点在轴上的位置), inner, axis, o }] }
  // 主轴 = 面积最大的回转面的轴; 圆柱轴向长按包围盒反算(整圆柱精确: ext_i = h·|a_i| + 2r·√(1 − a_i²))。
  function revShape(view, k) {
    var es = groupEntries(view, 'face', k);
    if (es.length > NAMES.bom.shapeMaxFaces) return null;
    var id = itemIdOf(view), planes = 0, rev = [];
    es.forEach(function (e) {
      var g = faceSig(e, id, k);
      if (g.surface === NAMES.planeSurface) { planes++; return; }
      var isCyl = g.surface === NAMES.cylinderSurface, isCone = g.surface === NAMES.coneSurface;
      if ((!isCyl && !isCone) || typeof g.radius !== 'number' || !g.axis || !g.axisOrigin || !g.center) return;
      var a = g.axis, o = g.axisOrigin, c = g.center;
      var w = [c[0] - o[0], c[1] - o[1], c[2] - o[2]], tc = vdot(w, a);
      var rad = [w[0] - tc * a[0], w[1] - tc * a[1], w[2] - tc * a[2]];
      var h = null;
      if (isCyl && g.bbox) {
        var b = g.bbox, i = [0, 1, 2].sort(function (p, q) { return Math.abs(a[q]) - Math.abs(a[p]); })[0];
        var ext = Math.abs(b[1][i] - b[0][i]), ai = Math.abs(a[i]);
        if (ai > 1e-6) h = sig7(Math.max(0, (ext - 2 * g.radius * Math.sqrt(Math.max(0, 1 - ai * ai))) / ai));
      }
      // 锥面的 radius 是参考半径(实测 2026-11-17: 沉头锥报的是小端 = 杆半径)→ 另给 rmax = 最大半径:
      // 取与轴最垂直的世界轴方向上的包围盒宽 / (2√(1 − a_i²))(轴与世界轴对齐时精确)
      var rmax;
      if (isCone && g.bbox) {
        var bb = g.bbox, j0 = [0, 1, 2].sort(function (p, q) { return Math.abs(a[p]) - Math.abs(a[q]); })[0];
        var s0 = Math.sqrt(Math.max(0, 1 - a[j0] * a[j0]));
        if (s0 > 1e-6) rmax = sig7(Math.abs(bb[1][j0] - bb[0][j0]) / (2 * s0));
      }
      var entry = { t: isCyl ? 'cyl' : 'cone', r: g.radius, h: h, tc: sig7(tc), inner: g.normal ? vdot(g.normal, rad) < 0 : false, axis: a, o: o,
                    area: isCyl && h ? 2 * Math.PI * g.radius * h : 0 };
      if (rmax !== undefined) entry.rmax = rmax;
      rev.push(entry);
    });
    var out = { faces: es.length, planes: planes, len: null, rev: rev };
    if (!rev.length) return out;
    var main = rev.slice().sort(function (p, q) { return q.area - p.area; })[0];
    // 总长: 显示网格全部顶点在主轴上的投影范围
    try {
      var fg = view[NAMES.shellGroups.face], mesh = fg && fg[NAMES.faceMesh], pos = mesh.geometry.attributes.position.array, me = mesh.matrixWorld && mesh.matrixWorld.elements;
      var lo = Infinity, hi = -Infinity;
      for (var j = 0; j + 2 < pos.length; j += 3) {
        var p = xfPoint(me, pos[j], pos[j + 1], pos[j + 2]), t = (p[0] * main.axis[0] + p[1] * main.axis[1] + p[2] * main.axis[2]) / k;
        if (t < lo) lo = t;
        if (t > hi) hi = t;
      }
      if (hi > lo) out.len = sig7(hi - lo);
    } catch (x) {}
    rev.forEach(function (r) { delete r.area; });
    return out;
  }

  // 两个指纹是不是同一种零件(手性另判)
  function samePrint(a, b) {
    var T = NAMES.bom.relTol;
    var close = function (x, y) { return Math.abs(x - y) <= T * Math.max(Math.abs(x), Math.abs(y), 1e-12); };
    if (!close(a.volume, b.volume) || !close(a.area, b.area)) return false;
    for (var i = 0; i < 3; i++) if (!close(a.moments[i], b.moments[i]) && Math.abs(a.moments[i] - b.moments[i]) > T * a.moments[2]) return false;
    var ka = Object.keys(a.faces).sort(), kb = Object.keys(b.faces).sort();
    return ka.join() === kb.join() && ka.every(function (t) { return a.faces[t] === b.faces[t]; });
  }

  // 物体所在组的路径(外 → 内; 不含根组)
  function groupPath(nodeKey) {
    var out = [], G = get('ed.groupsDb');
    if (!G) return out;
    var k = nodeKey;
    for (var guard = 0; guard < 32; guard++) {
      var g;
      try { g = invokeOn('groups.getParent', [k], G); } catch (e) { break; }
      if (!g || g[NAMES.bom.groupId] === NAMES.bom.rootGroupId) break;
      var gk = invokeOn('nodes.item2key', [g]);
      out.unshift(invokeOn('nodes.getName', [gk]));
      k = gk;
    }
    return out;
  }

  // args = { members?: [稳定 id](配置; 省略 = 当前可见实体), units? }
  // → { units, lines: [{ name, names, qty, members: [{id, name, path}], path, material, volume, area, faces, chiral, mirrorOf? }], skipped: [{id, name, err}] }
  def('bom.scan', 'bom.read', function (a) {
    a = a || {};
    var U = unitOf(a), k = U.k;
    var st = need('nodes.state'), id2key = new Map();
    st[NAMES.node2id].forEach(function (id, key) { id2key.set(id, key); });
    var want = new Set(Array.isArray(a.members) ? a.members.map(String) : visibleSolids().map(function (x) { return x.id; }));
    var pool = solidViews().filter(function (v) { return want.has(st[NAMES.node2id].get(invokeOn('nodes.item2key', [v]))); });
    // 材质: 稳定 id → 材质名
    var matName = {};
    try {
      var mn = materialsNow(), byUuid = {};
      mn.materials.forEach(function (d) { byUuid[d[NAMES.materialUuid]] = d.name; });
      mn.assign.forEach(function (p) { matName[p[0]] = byUuid[p[1]] || null; });
    } catch (e) {}
    var parts = [], skipped = [];
    pool.forEach(function (v) {
      var key = invokeOn('nodes.item2key', [v]), sid = st[NAMES.node2id].get(key), nm = nameOf(v);
      try { parts.push({ id: sid, name: nm, path: groupPath(key), fp: partPrint(v, k), material: matName[sid] || null, view: v }); }
      catch (e) { skipped.push({ id: sid, name: nm, err: errMsg(e) }); }
    });
    // 归并: 指纹相同且手性相同(或有一方分不清) → 同一行; 指纹相同手性相反 → 镜像件单列
    var lines = [];
    parts.forEach(function (p) {
      var hit = lines.filter(function (l) { return samePrint(l.fp, p.fp) && (l.fp.chiral === p.fp.chiral || l.fp.chiral === 0 || p.fp.chiral === 0); })[0];
      if (hit) { hit.members.push(p); return; }
      lines.push({ fp: p.fp, members: [p] });
    });
    lines.forEach(function (l, i) {
      var twin = lines.filter(function (o, j) { return j !== i && samePrint(o.fp, l.fp) && o.fp.chiral === -l.fp.chiral && l.fp.chiral !== 0; })[0];
      if (twin) l.mirrorOf = lines.indexOf(twin);
    });
    var common = function (arr) { var c = {}; arr.forEach(function (x) { c[x] = (c[x] || 0) + 1; }); return Object.keys(c).sort(function (x, y) { return c[y] - c[x]; })[0]; };
    var out = lines.map(function (l) {
      var names = l.members.map(function (p) { return p.name; });
      var paths = l.members.map(function (p) { return p.path.join(' / '); });
      // key: 零件身份(指纹的文本形式; 跨配置相同的零件 key 相同 → BOM 里的人工编辑跟着零件走)
      var fk = Object.keys(l.fp.faces).sort().map(function (t) { return t + l.fp.faces[t]; }).join('.');
      return { key: [l.fp.volume, l.fp.area, l.fp.moments.join('/'), fk, l.fp.chiral].join('|'),
               name: common(names), names: names.filter(function (x, i) { return names.indexOf(x) === i; }), qty: l.members.length,
               members: l.members.map(function (p) { return { id: p.id, name: p.name, path: p.path }; }),
               path: common(paths) || '', material: common(l.members.map(function (p) { return p.material || ''; })) || null,
               volume: l.fp.volume, area: l.fp.area, faces: l.fp.faces, chiral: l.fp.chiral, mirrorOf: l.mirrorOf,
               shape: (function () { try { return revShape(l.members[0].view, k); } catch (x) { return null; } })() };
    });
    // mirrorOf 指向排序前的下标 → 换成名字, 再按 组路径 → 名字 排
    out.forEach(function (l) { if (l.mirrorOf !== undefined) l.mirrorOf = out[l.mirrorOf].name; });
    out.sort(function (x, y) { return x.path < y.path ? -1 : x.path > y.path ? 1 : (x.name < y.name ? -1 : x.name > y.name ? 1 : 0); });
    return { units: U.unit, lines: out, skipped: skipped };
  });

  // ---------------------------------------------------------------------
  // D: 导出 STEP(出图用)。走 Plasticity 自己的导出: Editor.export() → 主进程"另存为"对话框(由 Rust 侧事先在主进程
  // 武装成一次性自动应答, 见 pla-bridge export.rs) → 站内 EXPORT STEP 对话框(格式 / 解析面转 NURBS / OK)→ 自带写出器。
  // 导出范围 = 当前选择; 没选 = 全部可见。所以这里先记下用户选择, 按稳定 id 选上要导的零件, 导完由 file.exportRestore 还原。
  // 注意: 写文件期间不能动选择(会取消导出命令), 还原要等 Rust 侧看到文件落盘之后。
  // args = { ids?: [稳定 id](省略 / 空 = 全部可见), format?: 'AP203' | 'AP214' | 'AP242', nurbs?: bool }
  // → { selected, format, prev(原选择条数), names(选上的零件名) }
  function exportStepBox(dlg) {
    var X = NAMES.exportStep, box = dlg;
    for (var i = 0; i < X.boxClimb && box; i++) {
      var t = box.innerText || '';
      if (t.indexOf(X.okText) >= 0 && t.indexOf(X.cancelText) >= 0) return box;
      box = box.parentElement;
    }
    return null;
  }
  function buttonByText(root, text) {
    return Array.prototype.slice.call(root.querySelectorAll('button')).filter(function (b) { return (b.innerText || '').trim() === text; })[0] || null;
  }
  function waitFor(test, ms) {
    return new Promise(function (resolve) {
      var t0 = Date.now();
      (function tick() { var v = test(); if (v || Date.now() - t0 > ms) return resolve(v || null); setTimeout(tick, 100); })();
    });
  }

  def('file.exportStep', 'file.export', function (a) {
    a = a || {};
    var X = NAMES.exportStep;
    if (document.querySelector(X.dialogTag)) throw new Error('an export dialog is already open in Plasticity');
    var s = need('sel.selected');
    // 记下用户当前选择(视图 / 拓扑对象本身), 导完还原
    var prev = [];
    X.selSets.forEach(function (k) { var set = s[k]; if (set && typeof set.forEach === 'function') set.forEach(function (v) { prev.push(v); }); });
    state.exportPrev = prev;
    // 选上要导的零件(稳定 id → 视图)
    var st = need('nodes.state'), map = st[NAMES.node2id];
    var want = new Set((Array.isArray(a.ids) ? a.ids : []).map(String));
    var pick = want.size ? solidViews().filter(function (v) { return want.has(map.get(invokeOn('nodes.item2key', [v]))); }) : [];
    if (want.size && pick.length !== want.size) throw new Error('export: ' + (want.size - pick.length) + ' of ' + want.size + ' ids not found');
    invokeOn('sel.removeAll', [], s);
    pick.forEach(function (v) { invokeOn('sel.add', [v], s); });
    invokeOn('ed.export');
    return waitFor(function () { return document.querySelector(X.dialogTag); }, X.dialogWaitMs).then(function (dlg) {
      if (!dlg) throw new Error('export: STEP dialog did not appear (save dialog not answered, or not a .step path)');
      var fmt = a.format || X.defaultFormat;
      if (X.formats.indexOf(fmt) < 0) throw new Error('export: unknown format ' + fmt);
      var fb = buttonByText(dlg, fmt);
      if (fb) fb.click();
      var cb = dlg.querySelector('input[name="' + X.nurbsInput + '"]');
      if (cb && a.nurbs !== undefined && !!cb.checked !== !!a.nurbs) cb.click();
      var box = exportStepBox(dlg), ok = box && buttonByText(box, X.okText);
      if (!ok) throw new Error('export: OK button not found');
      ok.click();
      return waitFor(function () { return !document.querySelector(X.dialogTag); }, X.closeWaitMs).then(function (closed) {
        if (!closed) throw new Error('export: STEP dialog did not close');
        // parts: 选上的零件名 + 体积 / 面积(文档单位, 与 bom.scan 同一指纹)。零件在组里时 Plasticity 会把全部可见实体都写进
        // STEP(实测 26.2.3), 而 FreeCAD 读进来不保留实体名 → 出图端按体积 + 面积挑实体(相同件挑哪个都一样)。
        var k = unitOf({}).k;
        var parts = pick.map(function (v) {
          var p = { name: nameOf(v) };
          try { var fp = partPrint(v, k); p.volume = fp.volume; p.area = fp.area; } catch (e) { p.err = errMsg(e); }
          return p;
        });
        return { selected: pick.length, format: fmt, prev: prev.length, names: parts.map(function (p) { return p.name; }), parts: parts };
      });
    });
  }, { async: true });

  // 还原 file.exportStep 之前的用户选择(Rust 侧确认文件已写出后调用)。
  def('file.exportRestore', 'file.export', function () {
    var s = need('sel.selected'), prev = state.exportPrev || [];
    invokeOn('sel.removeAll', [], s);
    var bad = 0;
    prev.forEach(function (v) { try { invokeOn('sel.add', [v], s); } catch (e) { bad++; } });
    state.exportPrev = null;
    var AM = NAMES.exportStep.autoMove, act = get('exec.activeCmd'), cancelled = false;
    if (!prev.length && act && act.constructor && act.constructor.name === AM.cmd && act[AM.agentKey] === AM.agent) {
      try { invokeOn('exec.cancel'); cancelled = true; } catch (e) {}
    }
    return { restored: prev.length - bad, failed: bad, cancelledAutoMove: cancelled };
  });

  def('err.last', 'err.hook', function () { return state.lastError; });

  def('tree.list', 'tree', function () {
    var out = [];
    eachVnode(function (v) {
      if (typeof v.type === 'function' && NAMES.outlinerSections.indexOf(v.type.name) >= 0 && v.props && v.props.object) {
        var o = v.props.object;
        out.push({ name: String(o.name || ''), ctor: (o.constructor && o.constructor.name) || '',
                   indent: v.props.indent || 0, selected: !!v.props.selected });
      }
      return undefined;
    });
    return out;
  });

  // ---------------------------------------------------------------------
  function invoke(name, args) {
    try {
      var api = API[name];
      if (!api) return { ok: false, err: 'unknown-api:' + name };
      ensureHooks();
      if (api.cap) {
        var c = capState(api.cap);
        if (!c.ok) return { ok: false, err: c.missing[0] === 'editor' ? 'cap:editor' : 'cap:' + api.cap, missing: c.missing, todo: c.todo };
      }
      var v = api.fn(args || {});
      // 异步接口(如 sel.apply 要等命令执行完): 返回 Promise, call() 再整体序列化。
      if (v && typeof v.then === 'function') {
        return v.then(function (x) { return { ok: true, v: x }; }, function (e) { return failure(api, e); });
      }
      return { ok: true, v: v };
    } catch (e) {
      return failure(api, e);
    }
  }

  function failure(api, e) {
    if (e instanceof CapError) return { ok: false, err: 'cap:' + (api ? api.cap : '?'), missing: e.missing };
    return { ok: false, err: 'exc:' + errMsg(e) };
  }

  var self = {
    api: API_VERSION,
    rev: CFG.rev,
    // 同步接口返回 JSON 字符串; 异步接口返回 Promise<JSON 字符串>(Runtime.evaluate 用 awaitPromise 取值)。
    call: function (name, args) {
      var r = invoke(name, args);
      return r && typeof r.then === 'function' ? r.then(JSON.stringify) : JSON.stringify(r);
    },
    // 批量只用于同步的只读轮询; 混进异步接口如实报错。
    batch: function (list) {
      var out = {};
      (list || []).forEach(function (it) {
        if (API[it[0]] && API[it[0]].async) { out[it[0]] = { ok: false, err: 'exc:async api in batch' }; return; }
        out[it[0]] = invoke(it[0], it[1]);
      });
      return JSON.stringify(out);
    },
    hasEditor: function () { return !!editor(); },
    // 委托命令回调的落点(见 ensureDelegate); 不是对外接口, Rust 不调用。
    _onCommand: function (action, el) { try { onCommand(action, el); } catch (e) {} },
    bindEditor: function (e) {
      if (!isEditor(e)) return false;
      W[CFG.editorGlobal] = e;
      return !!editor();
    },
    dispose: function () {
      disposeHooks();
      try { clearHighlight(); } catch (e) {} // 热替换时别把高亮网格留在场景里
      if (W.__PLA__ === self) delete W.__PLA__;
    }
  };

  W.__PLA__ = self;
  ensureHooks();
  return JSON.stringify({ installed: true, api: API_VERSION, rev: CFG.rev, editor: !!editor() });
})

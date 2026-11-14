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
    // ---- A2: 吸附点(测量 / 自由移动命令用的同一批点; SnapScene 按物体缓存, 新物体建完即有) ----
    'ed.snapScene':            ['ed', 'snapScene'],
    'snap.canonicals':         ['ss', 'canonicals'],
    'snap.occurrences':        ['ss', 'occurrences'],
    'fac.cornerBox':           ['fac', 'CornerBoxFactory'],
    'fac.multiFillet':         ['fac', 'MultiFilletShellFactory']
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
      point: 'GetPoint', ident: 'GetIdentifier'
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
    // 判定两平面"平行"的角度容差(度): 平行时 measure 额外给法向距离(壁厚); 平面的曲面类名
    parallelTolDeg: 0.1,
    planeSurface: 'Plane',
    // geo.query 每类最多返回的项数(超出如实报 truncated)
    queryLimit: 2000,
    // ---- A3: 事务回滚 ----
    // 撤销 = 在 document.body 上派发 edit:undo(与 Ctrl+Z 同一条路径; 异步, 实测 300 ms 内完成; 内部走 memento 恢复)
    undoCommand: 'edit:undo',
    undoPollMs: 50,
    undoStepTimeoutMs: 5000,
    // 每步命令的超时(ms); 按位置挑边/面的默认容差(文档单位)
    cmdTimeoutMs: 15000,
    pickTolerance: 0.5,
    // 配方表(唯一来源): 对外操作名 → 工厂表键 + 参数(对外名 → [工厂 setter, 类型]) + 必填 + 是否支持布尔。
    // 类型决定换算: point(文档单位 → 米, 构造 Vector3) / length / angle(度 → 弧度) / number / bool /
    //               edges / faces(拓扑 id 或 {near, of?, tol?}) / solids(物体 id / '$引用' / {name})。
    // 参数按这里的顺序设置。
    recipes: {
      box: { fac: 'fac.cornerBox', bool: true, required: ['p1', 'p2', 'height'],
             params: { p1: ['p1', 'point'], p2: ['p2', 'point'], height: ['height', 'length'] } },
      fillet: { fac: 'fac.multiFillet', required: ['edges', 'distance'],
                params: { edges: ['edges', 'edges'], distance: ['distance', 'length'] } }
    }
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
                           'cmd.selectionBaseHolder', 'sig.commandEnded', 'exec.cancel', 'sel.selected'] },
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
    'mat.state': { needs: ['ed.originator', 'nodes.state'], optional: ['mat.textures', 'tex.get'] },
    'mat.set':     { todo: 'N1' },
    'geo.read':  { needs: ['geo.items', 'geo.lookupTopo', 'geo.lookupItem', 'ed.docUnit'] },
    'hist.write': { needs: ['ed.undoStack', 'exec.cancel'] },
    'snap.read': { needs: ['geo.items', 'ed.docUnit', 'ed.snapScene', 'snap.canonicals'], optional: ['snap.occurrences'] },
    // 工厂类不列入: 首次使用前本来就没有(由代理按需挂上), 缺哪个在执行时按表键报
    'cmd.exec':    { needs: ['ed.enqueue', 'sig.commandEnded', 'exec.cancel', 'cmd.factoryBaseHolder', 'geo.items',
                             'geo.lookupTopo', 'geo.lookupItem', 'ed.docUnit', 'vp.camera'] },
    'bool.run':    { todo: 'N4' },
    'asset.place': { todo: 'N5' }
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
    return { size: typeof size === 'number' ? size : -1, items: items };
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
        this.plaEntries.forEach(function (en, i) {
          var hit = hitFor(en.view);
          onClick.call(cs, [hit], i === 0 && self.plaReplace ? NAMES.modReplace : NAMES.modAdd, 0);
          // 点击语义: 未介入实体上的面可能先选中整个实体 → 同一命令内再点一次下钻。
          if (idList(sel, en.kind).indexOf(String(en.id)) < 0) { drills++; onClick.call(cs, [hit], NAMES.modAdd, 0); }
        });
        this.plaReport = { drills: drills };
      }
    }
    state.selCmd = { base: Base, cls: PlaApplySelection };
    return PlaApplySelection;
  }

  // 入队一个命令并等它结束。editor.enqueue 的 Promise 要等整条命令链结束(后续命令等输入就永不兑现) →
  // 改为监听 commandEnded 认本命令; 入队前先取消活动命令(否则新命令排队不执行)。
  function runCommand(cmd, timeoutMs) {
    var sig = need('sig.commandEnded');
    try { invokeOn('exec.cancel'); } catch (e) {}
    return new Promise(function (resolve) {
      var done = false;
      var timer = null;
      var finish = function (how) { if (done) return; done = true; sig.remove(f); if (timer) clearTimeout(timer); resolve(how); };
      var f = function (c) { if (c === cmd) finish('ended'); };
      sig.add(f);
      timer = setTimeout(function () { finish('timeout'); }, timeoutMs || 10000);
      invokeOn('ed.enqueue', [cmd]);
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
      constructor(editor, build) { super(editor); this.plaBuild = build; this.plaResult = null; this.plaErr = null; }
      async execute() {
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
    }
    throw new Error('recipe: unknown param type ' + type);
  }

  // 一步 → build(cmd) 函数。参数在入队前全部换算好(选择器出错就不产生命令)。
  function buildStep(o, ctx) {
    var rc = NAMES.recipes[o.op];
    var Fac = need(rc.fac);
    var sets = [];
    Object.keys(rc.params).forEach(function (p) {
      if (o[p] === undefined) return;
      sets.push([rc.params[p][0], convertParam(rc.params[p][1], o[p], ctx)]);
    });
    if (o.bool !== undefined) {
      var targets = o.targets !== undefined ? convertParam('solids', o.targets, ctx) : [];
      if (!targets.length) throw new Error('bool needs targets');
      sets.push([NAMES.boolSetters.targets, targets]);
      sets.push([NAMES.boolSetters.op, NAMES.boolOps[o.bool]]);
    }
    return function (cmd) {
      var f = new Fac(cmd.editor);
      f[NAMES.facResource](cmd);
      sets.forEach(function (s) { f[s[0]] = s[1]; });
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

  def('hist.begin', 'hist.write', function () {
    state.tx = markNow();
    return { depth: state.tx.depth };
  });

  def('hist.commit', 'hist.write', function () {
    var had = !!state.tx;
    var d = had ? undoStack().length - state.tx.depth : 0;
    state.tx = null;
    return { committed: had, steps: d };
  });

  // 回滚当前事务(hist.begin 之后的全部步骤, 含期间用户自己的操作); 结束事务。
  def('hist.rollback', 'hist.write', function () {
    if (!state.tx) throw new Error('no transaction (call hist.begin first)');
    var mark = state.tx;
    return rollbackTo(mark).then(function (n) { state.tx = null; return { undone: n, depth: undoStack().length }; });
  }, { async: true });

  // 工厂挂载状态(只读): 供 Rust 判断要不要让代理拍快照、自检显示。
  def('cmd.factories', 'cmd.exec', function () {
    var F = W[CFG.factoriesGlobal];
    var keys = {};
    Object.keys(NAMES.recipes).forEach(function (op) { keys[NAMES.recipes[op].fac] = true; });
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
    NAMES.nodeVisSets.forEach(function (n) {
      want[n] = keysOf(V[n]);
      if (!sameSet(st[n], want[n])) visSame = false;
    });
    // 隔离层栈: 老存档(没有/格式不对)不动隔离
    var isoSaved = Array.isArray(V[NAMES.nodeIsolations]) && V[NAMES.nodeIsolations].every(Array.isArray) ? V[NAMES.nodeIsolations] : null;
    var isoWant = isoSaved ? isoSaved.map(keysOf).filter(function (l) { return l.length > 0; }) : null;
    var isoSame = true;
    if (isoWant) {
      var curIso = st[NAMES.nodeIsolations] || [];
      if (curIso.length !== isoWant.length || !isoWant.every(function (l, i) { return sameSet(toSet(curIso[i]), l); })) isoSame = false;
    }
    if (visSame && isoSame) return 'vis(same)';
    mementoGuard();
    var orig = need('ed.originator');
    var parts = [];
    if (!visSame) {
      // 隐藏 / 眼睛: 改 memento 的两项再 restore(场景随之重算)
      var m = invokeOn('orig.snapshot', [], orig);
      var nodes = {};
      Object.keys(m[NAMES.mementoNodes]).forEach(function (k) { nodes[k] = m[NAMES.mementoNodes][k]; });
      NAMES.nodeVisSets.forEach(function (n) { nodes[n] = Float64Array.from(want[n]); });
      m[NAMES.mementoNodes] = nodes;
      invokeOn('orig.restore', [m], orig);
      parts.push(want[NAMES.nodeVisSets[0]].length + '/' + want[NAMES.nodeVisSets[1]].length);
    }
    if (!isoSame) {
      // 隔离: memento 里的 isolations 恢复时被忽略(实测) → 用 NodeState 自己的方法: unisolate 清空全部层,
      // isolate(Set<nodeKey>) 压一层; 之后原样 restore 一次当前快照让场景重算(不重算则显示不变)。
      var nst = need('ed.nodes');
      invokeOn('nodes.unisolate', [], nst);
      isoWant.forEach(function (l) { invokeOn('nodes.isolate', [new Set(l)], nst); });
      invokeOn('orig.restore', [invokeOn('orig.snapshot', [], orig)], orig);
      parts.push('iso' + isoWant.length);
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
    var m = invokeOn('orig.snapshot', [], need('ed.originator'));
    var defs = plain(m[NAMES.mementoMaterials][NAMES.materialsList] || []);
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
      if (W.__PLA__ === self) delete W.__PLA__;
    }
  };

  W.__PLA__ = self;
  ensureHooks();
  return JSON.stringify({ installed: true, api: API_VERSION, rev: CFG.rev, editor: !!editor() });
})

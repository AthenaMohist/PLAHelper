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
  //        exec = 命令执行器；km = KeymapManager；kb = 键盘。
  //    方法类表键一律挂在其所属对象的根下, 调用时 this = 该对象(见 invokeOn)。
  // =====================================================================
  var LOC = {
    'ed.selection':            ['ed', 'selection'],
    'ed.viewports':            ['ed', 'viewports'],
    'ed.activeViewport':       ['ed', '_activeViewport', 'activeViewport'],
    'ed.click':                ['ed', 'click'],
    'click.addById':           ['click', 'addById'],
    'click.clear':             ['click', 'clear'],
    'ed.changeSelection':      ['ed', 'changeSelection'],
    'cs.onHover':              ['cs', 'onHover'],
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
    'vp.pointer':              ['vp', 'lastPointerEvent'],
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
    'oc.setNeedsUpdate':       ['oc', 'setNeedsUpdate']
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
    // 悬停命中项里携带拓扑对象的字段(按顺序取第一个存在的)与其版本 id 字段。
    hitItemKeys: ['topologyItem', 'object'],
    versionIdKey: 'versionId',
    // click.addById 的面 id 前缀(实测: '2:<versionId>' 选中面)。
    faceIdPrefix: '2:',
    // 扫描累积表上限(防异常增长)。
    scanMapLimit: 512,
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
    cameraExtraKeys: ['fov', 'zoom', 'near', 'far']
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
    'sel.write': { needs: ['click.addById', 'click.clear', 'exec.cancel'] },
    'hist.read': { needs: ['ed.undoStack', 'ed.geoVersion'] },
    'cam.read':  { needs: ['vp.camera'], optional: ['vp.orbit'].concat(orbitKeys) },
    'cam.write': { needs: ['vp.camera', 'vp.orbit', 'oc.quaternionEnd', 'oc.targetEnd', 'oc.radiusEnd', 'vp.render'] },
    'cam.nav':   { needs: ['vp.navigate'] },
    'cam.step':  { needs: ['oc.step'], optional: ['orbitSettings.step'] },
    'cam.roll':  { needs: ['reg.add', 'reg.listeners', 'oc.quaternionEnd', 'oc.step'], optional: ['oc.domElement'] },
    'scan':      { needs: ['cs.onHover', 'vp.pointer', 'exec.cancel', 'click.clear'] },
    'vp.state':  { needs: ['vp.camera', 'vp.orbit', 'ed.shading.mode', 'ed.shading.changed', 'ed.sections'],
                   optional: ['ed.shading.section', 'vp.cplane', 'vp.settings.renderer', 'vp.settings.ao', 'vp.settings.grid', 'vp.container'] },
    'keymap':    { needs: ['km.add', 'km.bindings'], optional: ['kb.update'] },
    'cmd.dispatch': { needs: ['reg.add', 'reg.listeners'] },
    'err.hook':  { needs: ['ed.errorSignal'] },
    'tree':      { needs: [], dom: true },
    'mat.set':     { todo: 'N1' },
    'cmd.exec':    { todo: 'N2' },
    'bool.run':    { todo: 'N4' },
    'asset.place': { todo: 'N5' }
  };

  // =====================================================================
  // 3. 解析器
  // =====================================================================
  var MISS = {};
  var state = { ed: null, finder: null, gen: 0, hooks: {}, lastError: null, hits: {}, pings: 0,
                scan: null, disposables: [], lastMaintain: 0 };

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
      case 'click': return get('ed.click');
      case 'cs': return get('ed.changeSelection');
      case 'exec': return get('ed.executor');
      case 'km': return get('ed.keymaps');
      case 'kb': return get('ed.keyboard');
      case 'orbitSettings': return get('ed.settings.orbit');
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

  // 调用方法类表键: this = 该表键所属的根对象(如 click.clear 的 this = ed.click)。缺失抛 CapError。
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
    try { endScan(); } catch (e) {}
    state.disposables.forEach(function (d) { try { d.dispose(); } catch (e) {} });
    state.disposables = [];
  }

  // =====================================================================
  // 7. 接口表（对外唯一入口；名字与 Rust shim::api 一一对应）
  // =====================================================================
  var API = {};
  function def(name, cap, fn) { API[name] = { cap: cap, fn: fn }; }

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
      set.forEach(function (x) { items.push({ kind: k[0], e: idElem(x) }); });
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
    call0(viewport(), 'vp.render');
    return { proj: proj, orbit: orbit };
  });

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
    if (set.has(el)) return false;
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

  // ---- 选择写入 / 取消命令 ----
  function clearBusyAttrs() {
    NAMES.bodyBusyAttrs.forEach(function (a) { try { document.body.removeAttribute(a); } catch (e) {} });
  }

  def('sel.cancelCommand', 'sel.write', function () {
    invokeOn('exec.cancel');
    clearBusyAttrs();
    return true;
  });
  def('sel.clear', 'sel.write', function () { invokeOn('click.clear'); return true; });
  // 按版本 id 临时选中一个面(遮挡补救用: 只为 navigateToSelection 取景, 不走点击管线)。
  def('sel.addFaceById', 'sel.write', function (a) {
    if (!a || a.versionId === undefined) throw new Error('sel.addFaceById needs {versionId}');
    invokeOn('click.addById', [NAMES.faceIdPrefix + a.versionId]);
    return true;
  });
  def('sel.faceIds', 'sel.read', function () {
    var s = need('sel.selected');
    var out = [];
    var set = get('sel.ids.face', s);
    var vk = NAMES.versionIdKey;
    if (set && typeof set.forEach === 'function') {
      set.forEach(function (x) { out.push(String(x && x[vk] !== undefined ? x[vk] : x)); });
    }
    var size = get('sel.size', s);
    return { selN: typeof size === 'number' ? size : -1, faces: out };
  });

  // ---- 悬停扫描(选择集应用): 包装 changeSelection.onHover, 页面侧自读指针坐标累积 id → 坐标 ----
  // 数据源铁律: 只信这张累积表; selection.hits 是陈旧的点击会话, 不可用(v4 教训)。
  function hitVersionId(h) {
    if (!h) return null;
    for (var i = 0; i < NAMES.hitItemKeys.length; i++) {
      var t = h[NAMES.hitItemKeys[i]];
      if (t && t[NAMES.versionIdKey] !== undefined) return String(t[NAMES.versionIdKey]);
    }
    return null;
  }

  function endScan() {
    var sc = state.scan;
    state.scan = null;
    if (sc && get('cs.onHover', sc.cs) === sc.wrapped) put('cs.onHover', sc.orig, sc.cs);
  }

  def('scan.begin', 'scan', function (a) {
    // 命令进行中悬停管线是暂停的 → 先取消活动命令、清 body 忙碌属性。
    try { invokeOn('exec.cancel'); } catch (e) {}
    if (!a || !a.keepSel) { try { invokeOn('click.clear'); } catch (e2) {} }
    clearBusyAttrs();
    var canvas = document.querySelector(CFG.viewportSelector + ' canvas');
    if (!canvas) throw new Error('no viewport canvas');
    var cs = need('ed.changeSelection');
    var cur = need('cs.onHover');
    if (!state.scan || state.scan.cs !== cs || cur !== state.scan.wrapped) {
      var orig = cur;
      var wrapped = function () {
        try {
          var m = state.scan && state.scan.map;
          var pe = get('vp.pointer');
          var hits = arguments[0];
          // onHover 自带的 x/y 参数恒为 0 → 自读视口最后一次指针事件(与 CDP Input 同一坐标系)。
          if (m && pe && typeof pe.clientX === 'number' && hits && hits.length) {
            var n = Object.keys(m).length;
            for (var i = 0; i < hits.length && n < NAMES.scanMapLimit; i++) {
              var vid = hitVersionId(hits[i]);
              if (vid && !m[vid]) { m[vid] = [Math.round(pe.clientX), Math.round(pe.clientY)]; n++; }
            }
          }
        } catch (e3) {}
        return orig.apply(cs, arguments);
      };
      put('cs.onHover', wrapped, cs);
      state.scan = { cs: cs, orig: orig, wrapped: wrapped, map: {} };
    }
    state.scan.map = {};
    var r = canvas.getBoundingClientRect();
    return [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)];
  });
  // 累积表: { versionId: [x, y] }(只记首次命中坐标)。
  def('scan.read', 'scan', function () {
    var m = state.scan && state.scan.map;
    var o = {};
    if (m) Object.keys(m).forEach(function (k) { o[k] = m[k]; });
    return o;
  });
  def('scan.clear', 'scan', function () { if (state.scan) state.scan.map = {}; return true; });
  // 结束扫描: 还原 onHover(不再常驻包装), 唤醒重绘。
  def('scan.end', 'scan', function () {
    endScan();
    call0(viewport(), 'vp.render');
    return true;
  });

  // ---- 视角 / 视口杂项 ----
  def('cam.navigateToSelection', 'cam.nav', function () { invokeOn('vp.navigate', [], viewport()); return true; });
  def('cam.render', 'cam.read', function () { invokeOn('vp.render', [], viewport()); return true; });
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
    put('oc.needsUpdate', true, oc);
    try { invokeOn('oc.setNeedsUpdate', [], oc); } catch (e) {}
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
    seg('renderer', function () { out.renderer = primFields(need('vp.settings.renderer'), NAMES.rendererKeys); });
    seg('ao', function () {
      var ao = need('vp.settings.ao');
      out.ao = primFields(ao, Object.keys(ao).filter(function (k) { return typeof ao[k] !== 'string'; }));
    });
    seg('grid', function () { out.grid = primFields(need('vp.settings.grid'), NAMES.gridKeys); });
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
    seg('shading', !!(S.shading && S.shading.mode), function () {
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
    seg('sections', Array.isArray(S.sections), function () {
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
    call0(viewport(), 'vp.render');
    return { applied: applied, missing: missing, failed: failed };
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
      return { ok: true, v: api.fn(args || {}) };
    } catch (e) {
      if (e instanceof CapError) return { ok: false, err: 'cap:' + api.cap, missing: e.missing };
      return { ok: false, err: 'exc:' + errMsg(e) };
    }
  }

  var self = {
    api: API_VERSION,
    rev: CFG.rev,
    call: function (name, args) { return JSON.stringify(invoke(name, args)); },
    batch: function (list) {
      var out = {};
      (list || []).forEach(function (it) { out[it[0]] = invoke(it[0], it[1]); });
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

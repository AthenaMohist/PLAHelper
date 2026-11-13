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
  //    根：ed = Editor；sel = 当前 Selection；vp = 视口（见 viewport()）；oc = 视口的轨道控制器。
  // =====================================================================
  var LOC = {
    'ed.selection':            ['ed', 'selection'],
    'ed.viewports':            ['ed', 'viewports'],
    'ed.activeViewport':       ['ed', '_activeViewport', 'activeViewport'],
    'ed.click.addById':        ['ed', 'click.addById'],
    'ed.click.clear':          ['ed', 'click.clear'],
    'ed.hover':                ['ed', 'changeSelection.onHover'],
    'ed.cancelCommand':        ['ed', 'executor.cancelActiveCommand'],
    'ed.undoStack':            ['ed', 'history._undoStack', 'history.undoStack'],
    'ed.geoVersion':           ['ed', 'geo._version', 'geo.version'],
    'ed.errorSignal':          ['ed', 'signals.environmentChanged'],
    'ed.registry.add':         ['ed', 'registry.add'],
    'ed.registry.listeners':   ['ed', 'registry.inlineListenersByCommandName'],
    'ed.keymaps.add':          ['ed', 'keymaps.add'],
    'ed.keymaps.bindings':     ['ed', 'keymaps.keyBindings'],
    'ed.keyboard.update':      ['ed', 'keyboard.updateKeymap'],
    'ed.settings.orbit':       ['ed', 'settings.OrbitControls'],
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
    'oc.update':               ['oc', 'update']
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
    vnodeScanLimit: 80000
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
    'sel.write': { needs: ['ed.click.addById', 'ed.click.clear', 'ed.cancelCommand'] },
    'hist.read': { needs: ['ed.undoStack', 'ed.geoVersion'] },
    'cam.read':  { needs: ['vp.camera'], optional: ['vp.orbit'].concat(orbitKeys) },
    'cam.write': { needs: ['vp.camera', 'vp.orbit', 'oc.quaternionEnd', 'oc.targetEnd', 'oc.radiusEnd', 'vp.render'] },
    'cam.nav':   { needs: ['vp.navigate'] },
    'cam.step':  { needs: ['oc.step'], optional: ['ed.settings.orbit'] },
    'cam.roll':  { needs: ['ed.registry.add', 'ed.registry.listeners', 'oc.quaternionEnd', 'oc.step'] },
    'scan':      { needs: ['ed.hover', 'vp.pointer', 'ed.cancelCommand', 'ed.click.clear'] },
    'vp.state':  { needs: ['vp.camera', 'vp.orbit', 'ed.shading.mode', 'ed.shading.changed', 'ed.sections'],
                   optional: ['ed.shading.section', 'vp.cplane', 'vp.settings.renderer', 'vp.settings.ao', 'vp.settings.grid', 'vp.container'] },
    'keymap':    { needs: ['ed.keymaps.add', 'ed.keymaps.bindings'], optional: ['ed.keyboard.update'] },
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
  var state = { ed: null, finder: null, gen: 0, hooks: {}, lastError: null, hits: {} };

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
  }

  function disposeHooks() {
    Object.keys(state.hooks).forEach(function (k) { try { state.hooks[k](); } catch (e) {} });
    state.hooks = {};
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
    bindEditor: function (e) {
      if (!isEditor(e)) return false;
      W[CFG.editorGlobal] = e;
      return !!editor();
    },
    dispose: function () {
      disposeHooks();
      if (W.__PLA_HOOK__ && W.__PLA_HOOK__.shim === self) delete W.__PLA_HOOK__;
      if (W.__PLA__ === self) delete W.__PLA__;
    }
  };

  // 兼容期别名（F1b 迁移完写路径后删除）：旧片段经 __PLA_HOOK__.findEditor 也走同一个获取入口。
  W.__PLA_HOOK__ = { shim: self, findEditor: function () { return editor(); } };
  W.__PLA__ = self;
  ensureHooks();
  return JSON.stringify({ installed: true, api: API_VERSION, rev: CFG.rev, editor: !!editor() });
})

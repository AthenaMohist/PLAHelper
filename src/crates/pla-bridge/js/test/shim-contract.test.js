// 垫片契约测试（由 Rust `shim::tests::shim_contract_node` 调用）。
// 用法: node shim-contract.test.js <安装表达式文件> <Rust 接口名 JSON> <Rust API_VERSION>
// 在 vm 沙箱里模拟页面 + 一个按 26.2.3 实测结构搭的假 Editor，逐项校验。
'use strict';
const fs = require('fs');
const vm = require('vm');
const assert = require('assert');

const [installFile, namesJson, apiVer, capsJson] = process.argv.slice(2);
const RUST_CAPS = JSON.parse(capsJson || '[]');
const INSTALL = fs.readFileSync(installFile, 'utf8');
const RUST_APIS = JSON.parse(namesJson);
const RUST_API_VERSION = Number(apiVer);

class V3 {
  constructor(x = 0, y = 0, z = 0) { this.x = x; this.y = y; this.z = z; }
  set(a, b, c) { this.x = a; this.y = b; this.z = c; return this; }
}
class Quat {
  constructor(x = 0, y = 0, z = 0, w = 1) { this.x = x; this.y = y; this.z = z; this.w = w; this.muls = []; }
  set(a, b, c, d) { this.x = a; this.y = b; this.z = c; this.w = d; return this; }
  setFromAxisAngle(axis, angle) { this.axis = axis; this.angle = angle; return this; }
  multiply(q) { this.muls.push(q.angle); return this; }
}
// R34d: 仿 THREE.Texture(只要垫片用到的部分); toJSON 仿 Texture.toJSON 的纯数据形态
class V2 { constructor() { this.x = 0; this.y = 0; } fromArray(a) { this.x = a[0]; this.y = a[1]; return this; } toArray() { return [this.x, this.y]; } }
class FakeTex {
  constructor(file) { this.userData = { filename: file }; this.repeat = new V2().fromArray([1, 1]); this.offset = new V2(); this.center = new V2(); this.rotation = 0; this.wrapS = 1001; this.wrapT = 1001; this.flipY = true; this.colorSpace = ''; }
  clone() { const t = new FakeTex(this.userData.filename); t.cloneOf = this; return t; }
  toJSON() { return { userData: this.userData, repeat: this.repeat.toArray(), offset: this.offset.toArray(), wrap: [this.wrapS, this.wrapT], flipY: this.flipY, colorSpace: this.colorSpace, image: 'img-uuid' }; }
}
// 深拷贝, 但 Texture 保持引用(真机 memento 里贴图是活对象)
function copyKeepTex(x) {
  if (x instanceof FakeTex) return x;
  if (Array.isArray(x)) return Array.from(x, copyKeepTex); // Array.from: 结果属于本 realm(沙箱数组 .map 仍是沙箱的 Array)
  if (x && typeof x === 'object') { const o = {}; for (const k of Object.keys(x)) o[k] = copyKeepTex(x[k]); return o; }
  return x;
}
function v3(x, y, z) { return new V3(x, y, z); }
function quat(x, y, z, w) { return new Quat(x, y, z, w); }

// 模拟 Plasticity CommandRegistry: 只有对象形式 add(target, {名: 回调}); 元素派发 CustomEvent 时执行。
function mkRegistry() {
  const listeners = new Map();
  const handlers = new Map();
  return {
    inlineListenersByCommandName: listeners,
    add(el, map) {
      if (arguments.length !== 2 || typeof map !== 'object') return; // 三参写法静默无效(与真机一致)
      for (const k of Object.keys(map)) {
        if (!listeners.has(k)) listeners.set(k, new Set());
        listeners.get(k).add(el);
        handlers.set(k + '@' + el.id, map[k]);
      }
      // 与真机一致: 返回 Disposable, dispose 即注销(热替换时旧实例会调用)
      const names = Object.keys(map);
      return { dispose() { names.forEach((k) => { const s = listeners.get(k); if (s) s.delete(el); handlers.delete(k + '@' + el.id); }); } };
    },
    fire(el, name) { const h = handlers.get(name + '@' + el.id); if (h) h(); },
  };
}

// 选择集合元素 → 版本 id 字符串(元素可能是 FaceId 对象或字符串)。
const faceKey = (f) => String(f && f.versionId !== undefined ? f.versionId : f);

function mkEditor(opts = {}) {
  const errListeners = new Set();
  const vp = {
    camera: { position: v3(1, 2, 3), quaternion: quat(0, 0, 0, 1), zoom: 1, fov: 50, near: 0.1, far: 1000, top: 3, aspect: 1.5 },
    orbitControls: {
      target: v3(0, 0, 0), targetEnd: v3(0, 0, 1), focalOffset: v3(0, 0, 0), focalOffsetEnd: v3(0, 0, 0),
      radius: 10, radiusEnd: 12, zoom: 1, zoomEnd: 1, quaternion: quat(0, 0, 0, 1), quaternionEnd: quat(0, 0, 0, 1),
      rotateDiscreteIncrement: 0.2618, updates: 0, update() { this.updates++; },
      domElement: { tag: 'canvas' }, needsUpdate: false, setNeedsUpdate() { this.needsUpdate = true; },
    },
    renders: 0, setNeedsRender() { this.renders++; }, navs: 0, navigateToSelection() { this.navs++; },
    lastPointerEvent: null,
    container: { getBoundingClientRect: () => ({ x: 1, y: 2, width: 300, height: 200 }) },
    _constructionPlane: { p: v3(0, 0, 0), n: v3(0, 0, 1), x: v3(1, 0, 0), y: v3(0, 1, 0) },
    settings: {
      Viewport: { renderer: { antiAlias: true, toneMapping: 4, toneMappingExposure: 1 }, ao: { enabled: true, radius: 2 } },
      Grid: { every: 10, size: 1 },
    },
    // ---- R33 v2: 视口自带 memento + render mode + 显示标量(仿 26.2.3) ----
    isXRay: false, showOverlays: true, isRenderMode: false, renderToggles: 0, inits: [],
    // 仿 26.2.3 实测: 视口 fov 不是视场角, 是投影开关(透视 1 / 正交 0); 写它(任何值)都会切成透视
    get fov() { return this.camMode === 'perspective' ? 1 : 0; },
    set fov(v) { this.camMode = 'perspective'; this.fovWrites = (this.fovWrites || 0) + 1; },
    squish: 1, curvatureScale: 0.1, g0Scale: 11, g1Scale: 10, g2Scale: 0.2,
    camMode: 'orthographic', perspToggles: 0,
    togglePerspective() { this.camMode = this.camMode === 'orthographic' ? 'perspective' : 'orthographic'; this.perspToggles++; },
    // A5: 截图(cam.thumbnail: needsRender + handleRender + 读 canvas); 渲染时记下相机状态供断言
    needsRender: false, renders2: [],
    handleRender() { const c = this.camera, o = this.orbitControls; this.renders2.push({ q: [c.quaternion.x, c.quaternion.y, c.quaternion.z, c.quaternion.w], zoom: o.zoom, target: [o.target.x, o.target.y, o.target.z], mode: this.camMode }); },
    renderer: { domElement: { width: 1200, height: 800, parentElement: null } },
    snapshot() {
      return { camera: { mode: this.camMode, position: v3(1, 2, 3), quaternion: { _x: 0, _y: 0, _z: 0, _w: 1 }, zoom: 64, fov: 35 },
               target: v3(0, 0, 0), focalOffset: v3(0, 0, 0), isXRay: this.isXRay, showOverlays: this.showOverlays,
               constructionPlane: { n: v3(0, 0, 1), o: v3(0, 0, 0) }, shading: { mode: 'uniform' } };
    },
    init(s) { this.inits.push(s); this.isXRay = s.isXRay; this.showOverlays = s.showOverlays; },
    toggleRenderMode() { this.isRenderMode = !this.isRenderMode; this.renderToggles++; },
  };
  class FaceId { constructor(id, versionId) { this.id = id; this.versionId = versionId; } toString() { return 'F' + this.id; } }
  // ---- 选择命令系统(仿 Plasticity): Command → SelectionCommand → DeselectAllCommand; enqueue 的 Promise 永不兑现 ----
  const commandEnded = { fns: new Set(), add(f) { this.fns.add(f); }, remove(f) { this.fns.delete(f); }, dispatch(c) { [...this.fns].forEach((f) => f(c)); } };
  class Command { constructor(editor) { this.editor = editor; } }
  class SelectionCommand extends Command {}
  class DeselectAllCommand extends SelectionCommand {
    async execute() { const s = this.editor.selection.selected; ['solidIds', 'sheetIds', 'faceIds', 'edgeIds', 'curveIds'].forEach((k) => s[k].clear()); }
  }
  // 几何库: 面(版本 id '<实体>f<序号>')与实体
  const box = { min: v3(0, 0, 0), getCenter(t) { t.set(1, 2, 3); return t; } };
  const faceViews = {};
  const solidViews = { 1: { isSolid: true, id: 1 }, 2: { isSolid: true, id: 2 }, 3: { isSolid: true, id: 3 }, 7: { isSolid: true, id: 7 } };
  ['1f10', '1f11', '2f20', '2f21', '3f30'].forEach((vid) => { faceViews[vid] = { isFace: true, versionId: vid, solid: vid.split('f')[0], parentItem: solidViews[vid.split('f')[0]], getBoundingBox: () => box }; });
  class Plane {}
  class Cylinder {}
  const ed = {
    selection: { selected: {
      size: 2,
      solidIds: new Set([7]), sheetIds: new Set(), faceIds: new Set([new FaceId(5, '14f2025')]),
      edgeIds: new Set(), segmentIds: new Set(), curveIds: new Set(), curveVertexIds: new Set(),
      groupIds: new Set(), planeIds: new Set(),
    }, signals: { commandEnded } },
    // N2: 几何命令类(自建工厂命令的基类 = 它的父类 Command)
    commands: { DeselectAllCommand, CornerBoxCommand: class CornerBoxCommand extends Command {} },
    _Command: Command,
    // 名字管理(仿 NodeState): getName(item2key(视图)); read = 可见性集合 + nodeKey → 稳定 id
    nodes: {
      item2key(view) { return view && view.id !== undefined ? 'k' + view.id : null; }, getName(key) { return key ? 'Body' + key.slice(1) : undefined; },
      read: { hidden: new Set([101]), invisible: new Set(), isolations: [], node2id: new Map([[101, 'idA'], [102, 'idB'], [103, 'idC']]),
              node2material: new Map([[101, 1], [102, 1]]) },
      // 隔离(仿 26.2.3): unisolate 清空全部层, isolate(Set) 压一层
      isolate(set) { this.read.isolations = this.read.isolations.concat([new Set(set)]); },
      unisolate() { this.read.isolations = []; },
    },
    // 材质库(memento 里的 materials 段)
    matDb: { version: 3, counter: 2, materials: [{ type: 'physical', id: 1, uuid: 'uM1', name: 'Steel', baseColorFactor: [0.3, 0.3, 0.3, 1], metallicFactor: 0.9 }] },
    // R34d: 材质库的纹理加载器(仿 26.2.3 TextureLoader.get(filename) → Promise<Texture>)
    materials: { textures: { loads: [], get(f) { this.loads.push(f); return Promise.resolve(new FakeTex(f)); } } },
    // memento: 只关心 nodes 段的可见性/材质字段与 materials 段; restore 把它们写回(仿"撤销"路径)
    originator: {
      restores: 0,
      snapshot() {
        const r = ed.nodes.read;
        return { geo: { untouched: true },
                 materials: copyKeepTex(ed.matDb),
                 nodes: { hidden: Float64Array.from(r.hidden), invisible: Float64Array.from(r.invisible), isolations: r.isolations.map((s) => Float64Array.from(s)),
                          materials: [Float64Array.from([...r.node2material.keys()]), Int32Array.from([...r.node2material.values()])] } };
      },
      restore(m) {
        this.restores++; this.last = m;
        ed.nodes.read.hidden = new Set(Array.from(m.nodes.hidden)); ed.nodes.read.invisible = new Set(Array.from(m.nodes.invisible));
        // 与真机一致: memento 里的 isolations 恢复时被忽略
        const [K, V] = m.nodes.materials; ed.nodes.read.node2material = new Map(Array.from(K).map((k, i) => [k, V[i]]));
        // 仿真机事故: 指派已写入后在材质段抛错(半套状态)
        if (this.failNext) { this.failNext = false; throw new TypeError('undefined is not a function'); }
        // 与真机一致: 贴图槽里必须是真 Texture(deserializeTexture 调它的方法), 纯 JSON 抛错
        m.materials.materials.forEach((d) => Object.keys(d).forEach((k) => {
          const t = d[k] && d[k].texture;
          if (t && typeof t.clone !== 'function') throw new TypeError('undefined is not a function');
        }));
        ed.matDb = copyKeepTex(m.materials);
      },
    },
    layers: { isShowingEdges: true, isShowingFaces: true, isShowingCurves: true, isShowingMeshes: true, isShowingImages: true,
              isShowingConstraints: true, isShowingControlPoints: false, isShowingContinuityLabels: true },
    document: { settings: {
      _Unit: { length: 'millimeter', angle: 'degree', mass: 'gram' },
      inits: [],
      cur: { Grid: { factor: 1, every: 10, size: 120 }, Display: { toneMapping: { type: 'ACES', exposure: 1 } } },
      snapshot() { return JSON.parse(JSON.stringify(this.cur)); },
      init(s) { this.inits.push(s); this.cur = JSON.parse(JSON.stringify(s)); },
    } },
    enqueue(cmd) {
      this.enqueued = (this.enqueued || 0) + 1;
      // 与真机一致: 命令失败(execute 抛错)也派发 commandEnded
      Promise.resolve().then(() => cmd.execute()).catch(() => {}).then(() => commandEnded.dispatch(cmd));
      return new Promise(() => {}); // 与真机一致: 整条命令链结束前不兑现
    },
    shading: {
      mode: { tag: 'Default', materials: 'mats', isTinted: false },
      _sectionMode: { tag: 'x' },
      signals: { shadingChanged: { n: 0, dispatch() { this.n++; } } },
      // v2: 着色 memento(matcap 是纹理, 来源文件在 userData.filename)
      cur: { matcap: '.././img/ceramic_lightbulb.exr', tint: true, shader: null, props: {}, env: '.././img/outdoor_cloudy_01_1k.exr' },
      matcapCalls: [], inits: [],
      environmentRotation: 0, environmentIntensity: 1,
      // 视口截面(仿 Section: 基准平面/位置 + transform(distance, angle1, angle2, flip))
      isSectioned: false,
      section: {
        originalPlane: { normal: v3(-1, 0, 0), constant: 0 }, originalPosition: v3(0, 0, 0),
        distance: 0, angle1: 0, angle2: 0, flip: false,
        transform(d, a1, a2, f) { this.distance = d; this.angle1 = a1; this.angle2 = a2; this.flip = f; },
      },
      snapshot() { const c = this.cur; return { mode: 'uniform', matcap: { userData: { filename: c.matcap } }, shader: c.shader ? { name: c.shader } : null, isTinted: c.tint, shaderProperties: Object.assign({}, c.props), environment: { userData: { filename: c.env } }, rotation: this.environmentRotation, intensity: this.environmentIntensity }; },
      init(s) { this.inits.push(s); this.cur.props = Object.assign({}, s.shaderProperties); },
      setMatcap(f, t) { this.matcapCalls.push([f, t]); this.cur.matcap = f; this.cur.tint = t; },
    },
    sections: { sections: [{ enabled: true, p: v3(0, 0, 1), n: v3(0, 0, -1), x: v3(1, 0, 0), y: v3(0, 1, 0) }] },
    viewports: new Set([vp]),
    history: opts.altHistory ? { undoStack: [1] } : { _undoStack: [1, 2, 3] },
    geo: {
      _version: 9,
      // N2: 物体列表 [{view, model}] 与工厂造出的拓扑项(版本 id → 视图)
      items: [], _topo: {}, _models: {},
      lookupTopologyItemById(vid) {
        if (this._topo[vid]) return { view: this._topo[vid], model: this._models[vid] || {} };
        return faceViews[vid] ? { view: faceViews[vid], model: { GetSurface: () => ({ surface: vid.endsWith('0') ? new Plane() : new Cylinder(), orientation: false }) }, topologyId: 1 } : null;
      },
      lookupItemById(id) {
        if (solidViews[id]) return { view: solidViews[id], model: {} };
        const it = this.items.find((x) => x.view.userData && x.view.userData.versionId === Number(id));
        return it || null;
      },
    },
    signals: {
      environmentChanged: {
        add(fn) { errListeners.add(fn); return { detach() { errListeners.delete(fn); } }; },
        remove(fn) { errListeners.delete(fn); },
      },
      documentSettingsChanged: { n: 0, dispatch() { this.n++; } },
    },
    // 点击处理(仿 ClickStrategy): 点未介入实体上的面 → 先选中整个实体; 再点一次 → 换成该面。
    changeSelection: {
      clicks: [],
      onClick(hits, mod) {
        const s = ed.selection.selected;
        const o = hits[0].object;
        this.clicks.push([o.versionId || o.id, mod]);
        if (mod === 0) ['solidIds', 'sheetIds', 'faceIds', 'edgeIds', 'curveIds'].forEach((k) => s[k].clear());
        if (o.isSolid) { s.solidIds.add(o.id); return; }
        const sid = Number(o.solid);
        const engaged = s.solidIds.has(sid) || [...s.faceIds].some((f) => faceKey(f).split('f')[0] === o.solid);
        if (!engaged && !opts.directFace) s.solidIds.add(sid);
        else { s.solidIds.delete(sid); s.faceIds.add(o.versionId); }
      },
    },
    executor: { cancels: 0, cancelActiveCommand() { this.cancels++; }, active: false, hasActiveCommand() { return this.active; } },
    // A2: 吸附缓存(仿 SnapScene: 'Item:<版本id>|false' → {snapsForItem, positions, box}; occurrences = [{entry, transform}])
    snapScene: { canonicals: new Map(), occurrences: [] },
    registry: mkRegistry(),
    keymaps: {
      added: [],
      keyBindings: [
        { selector: 'plasticity-viewport', keystrokes: 'left', command: 'orbit:rotate:left' },
        { selector: 'body', keystrokes: 'q', command: 'keep:me' },
      ],
      add(src, m) { this.added.push([src, m]); },
    },
    keyboard: { updates: 0, updateKeymap() { this.updates++; } },
    settings: { OrbitControls: { rotateDiscreteIncrement: 0.2618 } },
    _undo() {
      const st = this.history._undoStack || this.history.undoStack;
      const top = st.pop();
      this.undos = (this.undos || 0) + 1;
      if (top && top.snapshot) this.geo.items = top.snapshot;
    },
    _fireError(e) { errListeners.forEach((f) => f(e)); },
    _errCount() { return errListeners.size; },
  };
  if (opts.noClick) delete ed.enqueue;
  return ed;
}

// 视口元素: dispatchEvent 交给当前 editor 的注册表执行(模拟 Atom 命令分发)。
let currentEd = null;
const vpEl = {
  id: 'vp1',
  dispatchEvent(ev) { if (currentEd) currentEd.registry.fire(this, ev.type); return true; },
  contains() { return true; },
};
const canvasEl = { getBoundingClientRect: () => ({ x: 10.2, y: 20.7, width: 800.4, height: 600 }) };
// A3: body 上派发 edit:undo → 当前假编辑器异步撤一步(仿 Ctrl+Z: 弹出撤销栈顶, 几何退回该步之前的快照)
const body = {
  attrs: {}, removeAttribute(a) { delete this.attrs[a]; },
  dispatchEvent(ev) { if (ev.type === 'edit:undo' && currentEd && currentEd._undo) setTimeout(() => currentEd._undo(), 5); return true; },
};
class CustomEvent { constructor(type, init) { this.type = type; this.bubbles = !!(init && init.bubbles); } }
const ctx = vm.createContext({
  CustomEvent,
  setTimeout,
  clearTimeout,
  document: {
    body,
    getElementById: () => null,
    // A5: cam.thumbnail 用的离屏 canvas
    createElement: () => ({ width: 0, height: 0, getContext: () => ({ fillRect() {}, drawImage() {} }), toDataURL: () => 'data:image/png;base64,UE5H' }),
    querySelectorAll: () => [vpEl],
    querySelector: (sel) => (sel === 'plasticity-viewport' ? vpEl : sel === 'plasticity-viewport canvas' ? canvasEl : null),
  },
});
vm.runInContext('var window = this;', ctx);
const run = (code) => vm.runInContext(code, ctx);
const call = (name, args) => JSON.parse(run(`window.__PLA__.call(${JSON.stringify(name)}, ${JSON.stringify(args || {})})`));

let n = 0;
function t(name, fn) { fn(); n++; console.log('ok  ' + name); }

t('install', () => {
  const r = JSON.parse(run(INSTALL));
  assert.strictEqual(r.installed, true);
  assert.strictEqual(r.api, RUST_API_VERSION, 'JS API_VERSION != Rust');
  assert.strictEqual(JSON.parse(run(INSTALL)).same, true, '同 rev 重复注入应跳过');
});

t('接口名与 Rust 一一对应', () => {
  const js = call('core.apis').v.slice().sort();
  assert.deepStrictEqual(js, RUST_APIS.slice().sort());
});

t('无 editor 时返回 cap:editor, 不抛异常', () => {
  const r = call('sel.snapshot');
  assert.strictEqual(r.ok, false);
  assert.strictEqual(r.err, 'cap:editor');
  assert.strictEqual(call('nope.nope').err, 'unknown-api:nope.nope');
});

const ed = mkEditor();
ctx.__PLA_EDITOR__ = ed;
currentEd = ed;

t('探测: 完整结构下能力齐全, 预留项为 todo, tree 无 DOM 时失效', () => {
  const p = call('core.probe').v;
  assert.strictEqual(p.editor.ok, true);
  assert.strictEqual(p.editor.finder, 'global');
  for (const c of ['editor', 'sel.read', 'sel.write', 'hist.read', 'cam.read', 'cam.write', 'cam.nav', 'cam.step', 'cam.roll', 'sel.apply', 'vp.state', 'keymap', 'err.hook', 'cmd.exec']) {
    assert.strictEqual(p.caps[c].ok, true, c + ' 应可用: ' + JSON.stringify(p.caps[c]));
  }
  assert.strictEqual(p.caps['mat.set'].todo, 'N1');
  assert.strictEqual(p.caps.tree.ok, false);
});

t('能力名与 Rust shim::cap 一一对应', () => {
  const js = Object.keys(call('core.probe').v.caps).sort();
  assert.deepStrictEqual(js, RUST_CAPS.slice().sort());
});

t('sel.snapshot 保真', () => {
  const v = call('sel.snapshot').v;
  assert.strictEqual(v.size, 2);
  assert.deepStrictEqual(v.items[0], { kind: 'solid', e: 7, info: { name: 'Body7' } }, '物体带自身名字');
  assert.deepStrictEqual(v.items[1].info, {}, '查不到的面: 描述为空, 不报错');
  assert.strictEqual(v.items[1].kind, 'face');
  assert.strictEqual(v.items[1].e.versionId, '14f2025');
  assert.strictEqual(v.items[1].e.__ctor, 'FaceId');
  assert.strictEqual(v.items[1].e.__str, 'F5');
});

t('hist.state / cam.get', () => {
  assert.deepStrictEqual(call('hist.state').v, { undo: 3, geoVer: 9 });
  const c = call('cam.get').v;
  assert.strictEqual(c.px, 1);
  assert.strictEqual(c.qw, 1);
  assert.strictEqual(c.orbit.radiusEnd, 12);
  assert.deepStrictEqual(c.orbit.targetEnd, [0, 0, 1]);
});

t('cam.set: 相机 + 轨道控制器(当前值与 *End 两份)+ update; 回读一致', () => {
  const view = {
    px: 5, py: 6, pz: 7, qx: 0, qy: 1, qz: 0, qw: 0,
    orbit: { target: [1, 1, 1], radius: 20, radiusEnd: 21, zoom: 2 },
  };
  const r = call('cam.set', view);
  assert.strictEqual(r.ok, true, JSON.stringify(r));
  assert.strictEqual(r.v.orbit, true);
  const vp = [...ed.viewports][0];
  const oc = vp.orbitControls;
  assert.deepStrictEqual([oc.target.x, oc.targetEnd.x], [1, 1], 'target 与 targetEnd 都要写(缺省 End = 当前值)');
  assert.deepStrictEqual([oc.radius, oc.radiusEnd], [20, 21]);
  assert.deepStrictEqual([oc.zoom, oc.zoomEnd], [2, 2]);
  assert.strictEqual(oc.quaternionEnd.y, 1, '朝向同步给轨道控制器');
  assert.ok(oc.updates > 0, '写完要 update');
  const back = call('cam.get').v;
  assert.deepStrictEqual([back.px, back.qy, back.orbit.radiusEnd], [5, 1, 21]);
  // 老存档(只有 7 个数)也能用
  assert.strictEqual(call('cam.set', { px: 1, py: 2, pz: 3, qx: 0, qy: 0, qz: 0, qw: 1 }).v.orbit, false);
});

t('cmd.ping: 对象形式注册 → 派发 CustomEvent → 回调执行; 幂等', () => {
  const r1 = call('cmd.ping').v;
  assert.deepStrictEqual(r1, { registered: true, fired: true });
  const r2 = call('cmd.ping').v;
  assert.deepStrictEqual(r2, { registered: true, fired: true });
  assert.strictEqual(ed.registry.inlineListenersByCommandName.get('pla:shim:ping').size, 1, '不重复注册');
  assert.strictEqual(call('cmd.dispatch', {}).err.startsWith('exc:'), true, '缺 name 要报错');
});

t('cam.set 写完标记轨道控制器需要更新(否则缩放要等下次交互才生效)', () => {
  const oc = [...ed.viewports][0].orbitControls;
  oc.needsUpdate = false;
  call('cam.set', { px: 1, py: 2, pz: 3, qx: 0, qy: 0, qz: 0, qw: 1, orbit: { zoom: 3 } });
  assert.strictEqual(oc.needsUpdate, true);
  assert.deepStrictEqual([oc.zoom, oc.zoomEnd], [3, 3]);
});

t('cam.setOrbitStep / navigateToSelection / vpRect / render', () => {
  const vp = [...ed.viewports][0];
  assert.deepStrictEqual(call('cam.setOrbitStep', { rad: 0.5 }).v, { viewports: 1, settings: true });
  assert.strictEqual(vp.orbitControls.rotateDiscreteIncrement, 0.5);
  assert.strictEqual(ed.settings.OrbitControls.rotateDiscreteIncrement, 0.5, '内存设置一并改(防写回旧值)');
  call('cam.navigateToSelection');
  assert.strictEqual(vp.navs, 1);
  assert.deepStrictEqual(call('cam.vpRect').v, { x: 1, y: 2, width: 300, height: 200, scale: 1 });
  const r0 = vp.renders;
  call('cam.render');
  assert.strictEqual(vp.renders, r0 + 1);
});

t('滚转: 命令自动注册到视口元素; 派发/接口都按步进角绕本地 Z 转', () => {
  const vp = [...ed.viewports][0];
  const qe = vp.orbitControls.quaternionEnd;
  assert.ok(ed.registry.inlineListenersByCommandName.get('pla:roll:left').has(vpEl), '常驻维护已注册');
  vpEl.dispatchEvent(new CustomEvent('pla:roll:left', { bubbles: true }));
  vpEl.dispatchEvent(new CustomEvent('pla:roll:right', { bubbles: true }));
  assert.deepStrictEqual(qe.muls, [-0.5, 0.5], '步进角现读(上一项改成 0.5)');
  assert.strictEqual(vp.orbitControls.needsUpdate, true);
  assert.strictEqual(call('cam.roll', { sign: -1 }).v.step, 0.5);
  assert.deepStrictEqual(qe.muls.slice(-1), [-0.5]);
});

t('keymap.setBindings: 删本选择器下现行/旧版绑定, 不碰别人的, enable 时再加', () => {
  const r = call('keymap.setBindings', {
    enable: true, source: 'pla-helper',
    bindings: { left: 'orbit:rotate:right', 'alt-left': 'pla:roll:left' },
    legacy: { left: 'orbit:rotate:left' },
  }).v;
  assert.deepStrictEqual(r, { enabled: true, removed: 1 });
  assert.deepStrictEqual(ed.keymaps.keyBindings.map((b) => b.command), ['keep:me']);
  // vm 内建的对象原型与外层不同 → 经 JSON 比结构。
  assert.deepStrictEqual(JSON.parse(JSON.stringify(ed.keymaps.added.slice(-1))), [['pla-helper',
    { 'plasticity-viewport': { left: 'orbit:rotate:right', 'alt-left': 'pla:roll:left' } }]]);
  assert.ok(ed.keyboard.updates > 0);
  const n = ed.keymaps.added.length;
  call('keymap.setBindings', { enable: false, bindings: {}, legacy: {} });
  assert.strictEqual(ed.keymaps.added.length, n, 'enable=false 不加');
});

t('vp.save / vp.restore: R8-1 存档格式; 原样恢复全部应用; 切着色模式走派发链', () => {
  const s = call('vp.save').v;
  assert.deepStrictEqual(s.failed, []);
  assert.strictEqual(s.err, '');
  assert.deepStrictEqual(s.shading, { mode: 'Default', section: 'x' });
  assert.deepStrictEqual(s.sections[0].n, [0, 0, -1]);
  assert.deepStrictEqual(s.cplane.n, [0, 0, 1]);
  assert.deepStrictEqual(s.renderer, { antiAlias: true, toneMapping: 4, toneMappingExposure: 1 });
  assert.deepStrictEqual(s.grid, { every: 10, size: 1 });
  assert.strictEqual(s.cam.near, 0.1);
  assert.strictEqual(s.orbit.radiusEnd, 21, '前面 cam.set 写的值');
  // 本用例验证**老格式**存档(R33 之前存的)照常恢复: 去掉 v2 字段
  const old = JSON.parse(JSON.stringify(s));
  for (const k of ['v', 'view', 'renderMode', 'scalars', 'layers', 'shading2', 'doc', 'vis']) delete old[k];
  const d0 = ed.shading.signals.shadingChanged.n;
  const r = call('vp.restore', old).v;
  assert.deepStrictEqual(r.failed, []);
  for (const k of ['cam', 'orbit', 'shading', 'sections:1', 'cplane', 'renderer', 'ao', 'grid']) assert.ok(r.applied.includes(k), k);
  assert.ok(r.missing.includes('zoom(正交未应用)'), '非透视相机如实标注');
  assert.strictEqual(ed.shading.signals.shadingChanged.n, d0, '同模式不派发切换');
  const s2 = JSON.parse(JSON.stringify(old));
  s2.shading.mode = 'Matcap';
  s2.sections = [];
  call('vp.restore', s2);
  assert.strictEqual(ed.shading.mode.tag, 'Matcap');
  assert.strictEqual(ed.shading.mode.prev.tag, 'Default', '构造合法 prev');
  assert.strictEqual(ed.shading.signals.shadingChanged.n, d0 + 1);
  assert.strictEqual(ed.sections.sections[0].enabled, false, '存档没有的剖面禁用');
  const r3 = call('vp.restore', {}).v;
  assert.ok(r3.missing.includes('cam') && r3.applied.length === 0, '空存档如实报缺失');
});

t('R33 vp v2: 存全显示状态; 可见性走 memento(稳定 id); render mode / 网格 / 开关 / 标量 / 缩放 都恢复', () => {
  const vp = [...ed.viewports][0];
  const s = call('vp.save').v;
  assert.strictEqual(s.v, 2);
  assert.deepStrictEqual(s.failed, []);
  assert.strictEqual(s.view.camera.mode, 'orthographic');
  assert.deepStrictEqual(s.view.camera.quaternion, { x: 0, y: 0, z: 0, w: 1 }, '四元数 _x 形式也能读');
  assert.strictEqual(s.view.camera.zoom, vp.orbitControls.zoomEnd, '缩放存轨道控制器的值');
  assert.deepStrictEqual(s.vis, { hidden: ['idA'], invisible: [], isolations: [] }, '隐藏按稳定 id 存');
  assert.strictEqual(s.shading2.matcap, '.././img/ceramic_lightbulb.exr');
  assert.strictEqual(s.shading2.button, 'ceramic_lightbulb');
  assert.strictEqual(s.doc.Grid.size, 120);
  assert.strictEqual(s.layers.isShowingControlPoints, false);
  assert.strictEqual(s.scalars.g0Scale, 11);

  // 原样恢复: 可见性不变 → 不碰 memento; 着色不变 → 不点按钮
  const r0 = call('vp.restore', s).v;
  assert.deepStrictEqual(r0.failed, []);
  assert.ok(r0.applied.includes('vis(same)') && r0.applied.includes('shading(same)'), JSON.stringify(r0.applied));
  assert.ok(r0.applied.includes('doc(same)') && ed.document.settings.inits.length === 0, '文档设置没变就不写(否则文件变"未保存")');
  // 存档经 Rust 转一圈键序变了(serde_json 按字母排) → 仍是 doc(same), 不写
  const revK = (x) => Array.isArray(x) ? x.map(revK) : (x && typeof x === 'object' ? Object.fromEntries(Object.keys(x).reverse().map((k) => [k, revK(x[k])])) : x);
  const sr = JSON.parse(JSON.stringify(s)); sr.doc = revK(sr.doc);
  assert.ok(call('vp.restore', sr).v.applied.includes('doc(same)') && ed.document.settings.inits.length === 0, '键序不同不算改动');
  assert.strictEqual(ed.originator.restores, 0);

  // 改一组状态再恢复
  const B = JSON.parse(JSON.stringify(s));
  B.vis.hidden = ['idB', 'idC', 'idGone'];
  B.renderMode = true;
  B.doc.Grid.size = 60;
  B.layers.isShowingEdges = false;
  B.scalars.squish = 1.3;
  B.view.isXRay = true;
  B.view.camera.zoom = 99;
  const dsig = ed.signals.documentSettingsChanged.n;
  const r = call('vp.restore', B).v;
  assert.deepStrictEqual(r.failed, [], JSON.stringify(r));
  assert.strictEqual(ed.originator.restores, 1, '可见性走一次 memento restore');
  assert.deepStrictEqual([...ed.nodes.read.hidden].sort(), [102, 103]);
  assert.ok(ed.originator.last.geo.untouched, 'memento 其余部分取当下快照, 原样带回');
  assert.ok(r.missing.includes('node:idGone'), '找不到的节点如实报告');
  assert.strictEqual(vp.isRenderMode, true);
  assert.ok(!r.applied.some((a) => a.startsWith('shading')) && !r.missing.includes('shading2'), 'render mode 下不动着色, 也不算缺失');
  assert.ok(!r.missing.some((m) => ['cam', 'orbit', 'renderer', 'ao', 'grid'].includes(m)), 'v2 存档不报旧格式段落缺失: ' + JSON.stringify(r.missing));
  assert.strictEqual(ed.document.settings.cur.Grid.size, 60);
  assert.strictEqual(ed.signals.documentSettingsChanged.n, dsig + 1);
  assert.strictEqual(ed.layers.isShowingEdges, false);
  assert.strictEqual(vp.squish, 1.3);
  assert.strictEqual(vp.isXRay, true, 'vp.init 收到改过的 snapshot');
  assert.strictEqual(vp.orbitControls.zoom, 99);
  assert.strictEqual(vp.orbitControls.zoomEnd, 99, '缩放写两份(阻尼)');
  // 透视/正交: 当前被切成透视 → 恢复正交存档时用 togglePerspective 切回
  vp.camMode = 'perspective';
  const tg = vp.perspToggles;
  const rP = call('vp.restore', B).v;
  assert.ok(rP.applied.includes('projection'), JSON.stringify(rP.applied));
  assert.strictEqual(vp.camMode, 'orthographic', '切完之后的标量段不能再写 fov(会切回透视, 用户实测 2026-11-15)');
  assert.strictEqual(vp.perspToggles, tg + 1);
  assert.ok(!vp.fovWrites, '恢复全程不写视口 fov');
  assert.ok(!call('vp.restore', B).v.applied.includes('projection'), '模式相同不切');

  // 回到 A: render mode 关回去 → 刚关 render, 面板下一帧才出来 → 着色排队重试(不立即 setMatcap)
  ed.shading.cur.matcap = '.././img/gradient.exr';
  const rA = call('vp.restore', s).v;
  assert.deepStrictEqual(rA.failed, [], JSON.stringify(rA));
  assert.strictEqual(vp.isRenderMode, false);
  assert.ok(rA.applied.includes('shading:pending ceramic_lightbulb'), JSON.stringify(rA.applied));
  // render 已关、面板上就是没有这个按钮(自定义 matcap 等)且当前非 shader → 兜底 setMatcap
  const rA2 = call('vp.restore', s).v;
  assert.deepStrictEqual(rA2.failed, [], JSON.stringify(rA2));
  assert.deepStrictEqual(ed.shading.matcapCalls.slice(-1)[0], ['.././img/ceramic_lightbulb.exr', true]);
  assert.deepStrictEqual([...ed.nodes.read.hidden], [101]);

  // shader 模式下绝不直接 setMatcap(会让渲染循环停摆): 没按钮时如实报失败
  ed.shading.cur.shader = 'zebraVertical';
  const calls = ed.shading.matcapCalls.length;
  const rS = call('vp.restore', s).v;
  // 面板按钮暂时找不到 → 排队重试(或无 DOM 时如实失败); 关键是绝不直接 setMatcap
  assert.ok(rS.failed.some((f) => f.startsWith('shading2:')) || rS.applied.some((a) => a.startsWith('shading:pending')), JSON.stringify(rS));
  assert.strictEqual(ed.shading.matcapCalls.length, calls);
  ed.shading.cur.shader = null;

  // 截面: 存 开关 + 基准平面/位置 + 4 个参数; 恢复走 transform + isSectioned
  const sh = ed.shading;
  sh.isSectioned = true;
  sh.section.originalPlane.normal.set(0, 0, 1); sh.section.originalPlane.constant = -0.002;
  sh.section.transform(0.004, 0.3, 0.1, true);
  const sSec = call('vp.save').v;
  assert.deepStrictEqual(sSec.section2, { on: true, basePlane: { n: { x: 0, y: 0, z: 1 }, c: -0.002 }, basePos: { x: 0, y: 0, z: 0 }, distance: 0.004, angle1: 0.3, angle2: 0.1, flip: true });
  sh.isSectioned = false; sh.section.transform(0, 0, 0, false); sh.section.originalPlane.normal.set(-1, 0, 0); sh.section.originalPlane.constant = 0;
  const rSec = call('vp.restore', sSec).v;
  assert.ok(rSec.applied.includes('section:on'), JSON.stringify(rSec.applied));
  assert.ok(!rSec.applied.some((a) => a.startsWith('sections:')), 'v2 存档不走旧 SectionDatabase 段');
  assert.strictEqual(sh.isSectioned, true);
  assert.deepStrictEqual([sh.section.distance, sh.section.angle1, sh.section.angle2, sh.section.flip], [0.004, 0.3, 0.1, true]);
  assert.strictEqual(sh.section.originalPlane.normal.z, 1);
  assert.strictEqual(sh.section.originalPlane.constant, -0.002);
  const sOff = JSON.parse(JSON.stringify(sSec)); sOff.section2.on = false;
  assert.ok(call('vp.restore', sOff).v.applied.includes('section:off'));
  assert.strictEqual(sh.isSectioned, false);

  // 渲染环境: 存 文件/旋转/强度; 只在存档 render mode 开时恢复; 换环境要点缩略图(无 DOM → 如实失败), 旋转/强度直接写
  assert.deepStrictEqual(sSec.env, { file: '.././img/outdoor_cloudy_01_1k.exr', rotation: 0, intensity: 1 });
  const sEnv = JSON.parse(JSON.stringify(sSec)); sEnv.renderMode = true; sEnv.env.rotation = 1.2; sEnv.env.intensity = 0.7;
  const rEnv = call('vp.restore', sEnv).v;
  assert.ok(rEnv.applied.includes('env(same)'), JSON.stringify(rEnv));
  assert.deepStrictEqual([sh.environmentRotation, sh.environmentIntensity], [1.2, 0.7]);
  sEnv.env.file = '.././img/kloofendal_puresky_1k.exr';
  // 缩略图还没出现(刚开 render mode)→ 排队重试, 不报失败; 按钮名去掉 _1k 后缀
  assert.ok(call('vp.restore', sEnv).v.applied.includes('env:pending kloofendal_puresky'), '按钮名去掉 _1k 后缀, 找不到先排队重试');
  sh.environmentRotation = 0; sh.environmentIntensity = 1;
  // 用户场景: 从 render mode 状态切到非 render 的 shader 状态 → 关 render 后面板下一帧才出来, 着色排队重试而不是失败
  const sIso2 = JSON.parse(JSON.stringify(sSec)); sIso2.renderMode = false; sIso2.shading2 = Object.assign({}, sSec.shading2, { shader: 'isophotes', button: 'isophotes', matcap: '.././img/basic_side.exr', tint: false });
  assert.strictEqual(vp.isRenderMode, true, '前提: 当前在 render mode');
  const rSw = call('vp.restore', sIso2).v;
  assert.ok(rSw.applied.includes('shading:pending isophotes'), JSON.stringify(rSw));
  assert.ok(!rSw.missing.includes('env'), 'render mode 关的存档不报 env 缺失');
  call('vp.restore', sSec); // render mode 关回去

  // memento 恢复前先取消进行中的命令(26.2.3 默认命令常驻, 不能据此拒绝)
  const c0 = ed.executor.cancels;
  call('vp.restore', B);
  assert.ok(ed.executor.cancels > c0, '恢复可见性前取消活动命令');
  // 隔离: 两层(外层 idA+idB, 内层 idA)按稳定 id 存, 恢复走 memento 写回层栈
  call('vp.restore', s); // 先回到 A(无隔离)
  ed.nodes.read.isolations = [new Set([101, 102]), new Set([101])];
  const sIso = call('vp.save').v;
  assert.deepStrictEqual(sIso.vis.isolations, [['idA', 'idB'], ['idA']], '隔离层栈按稳定 id 存');
  ed.nodes.read.isolations = [];
  const rIso = call('vp.restore', sIso).v;
  assert.ok(rIso.applied.includes('vis:iso2'), JSON.stringify(rIso.applied));
  assert.ok(ed.originator.last && ed.originator.restores > 0, '隔离后原样 restore 一次让场景重算');
  assert.strictEqual(JSON.stringify(Array.from(ed.nodes.read.isolations, (x) => [...x].sort())), '[[101,102],[101]]');
  assert.ok(call('vp.restore', sIso).v.applied.includes('vis(same)'), '层栈相同不动');
  const rNoIso = call('vp.restore', s).v; // A 存档里隔离为空 → 解除
  assert.ok(rNoIso.applied.some((a) => a.startsWith('vis:')), JSON.stringify(rNoIso.applied));
  assert.strictEqual(ed.nodes.read.isolations.length, 0);
  // 窗口不可见(显示器关/被遮挡)时不做 memento 恢复(实测会在 Plasticity 内部抛错)
  run('document.visibilityState = "hidden"');
  const rH = call('vp.restore', B).v;
  assert.ok(rH.failed.some((f) => f.startsWith('vis:window hidden')), JSON.stringify(rH));
  run('delete document.visibilityState');
});

t('错误钩子: 安装、捕获、不重复挂', () => {
  assert.strictEqual(call('err.last').v, null);
  call('err.last');
  assert.strictEqual(ed._errCount(), 1, '钩子只能挂一次');
  ed._fireError(new Error('reading active'));
  assert.strictEqual(call('err.last').v.msg, 'reading active');
});

t('batch 每项独立成败', () => {
  const r = JSON.parse(run(`window.__PLA__.batch([["hist.state",{}],["tree.list",{}]])`));
  assert.strictEqual(r['hist.state'].ok, true);
  assert.strictEqual(r['tree.list'].ok, false);
  assert.strictEqual(r['tree.list'].err, 'cap:tree');
});

t('F1b: 兼容别名 __PLA_HOOK__ 已删除, 页面上只有 __PLA__ 一个入口', () => {
  assert.strictEqual(run('typeof window.__PLA_HOOK__'), 'undefined');
  assert.strictEqual(run('typeof window.__PLA__.call'), 'function');
});

t('editor 换代(重载): 旧钩子拆除, 新钩子挂上', () => {
  const ed2 = mkEditor({ altHistory: true, noClick: true });
  ctx.__PLA_EDITOR__ = ed2;
  currentEd = ed2;
  assert.deepStrictEqual(call('hist.state').v, { undo: 1, geoVer: 9 });
  assert.strictEqual(ed._errCount(), 0, '旧 editor 的监听应拆除');
  assert.strictEqual(ed2._errCount(), 1);
  const p = call('core.probe').v;
  assert.strictEqual(p.editor.gen, 2);
  assert.strictEqual(p.altPaths['ed.undoStack'], 'history.undoStack', '应报告走了备选路径');
  assert.strictEqual(p.caps['sel.apply'].ok, false);
  assert.ok(p.caps['sel.apply'].missing.includes('ed.enqueue'));
});

t('不同 rev 注入 = 热替换(dispose 旧实例)', () => {
  const ed2 = ctx.__PLA_EDITOR__;
  const other = INSTALL.replace(/"rev":"[0-9a-f]+"/, '"rev":"ffffffffffffffff"');
  assert.notStrictEqual(other, INSTALL);
  assert.strictEqual(JSON.parse(run(other)).installed, true);
  call('err.last');
  assert.strictEqual(ed2._errCount(), 1, '热替换后监听仍只有一个');
  assert.strictEqual(run('window.__PLA__.rev'), 'ffffffffffffffff');
});

t('热替换后已注册的命令回调落到**当前**实例(委托式, 不闭包旧实例)', () => {
  // 上一项已热替换; editor 是 ed2(新注册表)。ping / 滚转都应由当前实例执行。
  assert.strictEqual(call('cmd.ping').v.fired, true);
  const before = JSON.parse(run(INSTALL.replace(/"rev":"[0-9a-f]+"/, '"rev":"eeeeeeeeeeeeeeee"'))).installed;
  assert.strictEqual(before, true, '再热替换一次');
  assert.strictEqual(call('cmd.ping').v.fired, true, '旧实例注册的委托回调, 计数进了新实例');
  assert.strictEqual(currentEd.registry.inlineListenersByCommandName.get('pla:shim:ping').size, 1, '不重复注册');
});

// ---- 异步接口(sel.apply / sel.clear 要等选择命令执行完): call() 返回 Promise<JSON> ----
async function ta(name, fn) { await fn(); n++; console.log('ok  ' + name); }
const callAsync = async (name, args) => JSON.parse(await run(`window.__PLA__.call(${JSON.stringify(name)}, ${JSON.stringify(args || {})})`));

(async () => {
  const ed3 = mkEditor();
  ctx.__PLA_EDITOR__ = ed3;
  currentEd = ed3;
  const s = ed3.selection.selected;

  await ta('sel.apply: 一个选择命令内完成; 跨实体的面先选中实体再下钻; 实体成员直接选中', async () => {
    const r = await callAsync('sel.apply', { items: [
      { kind: 'face', id: '1f10' }, { kind: 'face', id: '2f20' }, { kind: 'solid', id: '3' }, { kind: 'face', id: '2f21' },
    ] });
    assert.strictEqual(r.ok, true, JSON.stringify(r));
    assert.strictEqual(ed3.enqueued, 1, '整个选择集只入队一个命令(一个撤销步)');
    assert.deepStrictEqual([...s.faceIds].map(faceKey).sort(), ['1f10', '2f20', '2f21']);
    assert.deepStrictEqual([...s.solidIds], [3], '原有的实体 7 被替换掉; 下钻后实体 1/2 不残留');
    assert.deepStrictEqual(r.v.missing, []);
    assert.deepStrictEqual(r.v.extra, []);
    assert.ok(r.v.drills >= 2, '1f10 与 2f20 都要下钻: ' + r.v.drills);
    assert.strictEqual(r.v.how, 'ended', '靠 commandEnded 认结束, 不等 enqueue 的 Promise');
    assert.strictEqual(ed3.changeSelection.clicks[0][1], 0, '首项替换');
    assert.ok(ed3.changeSelection.clicks.slice(1).every((c) => c[1] === 1), '其余追加');
    // R24: 选择快照带可读描述(所属物体名 + 曲面类型), 界面靠它区分同类项
    const snap = JSON.parse(JSON.stringify(call('sel.snapshot').v.items));
    const f10 = snap.find((i) => i.e === '1f10');
    const f21 = snap.find((i) => i.e === '2f21');
    assert.deepStrictEqual(f10.info, { ownerName: 'Body1', surface: 'Plane' });
    assert.deepStrictEqual(f21.info, { ownerName: 'Body2', surface: 'Cylinder' });
    assert.deepStrictEqual(snap.find((i) => i.kind === 'solid').info, { name: 'Body3' });
  });

  await ta('sel.apply: 查不到的 id → missing, 不支持的类型 → unsupported; replace=false 保留现有选择', async () => {
    const r = await callAsync('sel.apply', { replace: false, items: [
      { kind: 'face', id: '3f30' }, { kind: 'face', id: '9f99' }, { kind: 'group', id: 'g1' },
    ] });
    assert.deepStrictEqual(r.v.missing, ['face:9f99']);
    assert.deepStrictEqual(r.v.unsupported, ['group:g1']);
    assert.deepStrictEqual([...s.faceIds].map(faceKey).sort(), ['1f10', '2f20', '2f21', '3f30'], '追加不清空');
    const none = await callAsync('sel.apply', { items: [{ kind: 'face', id: 'nope' }] });
    assert.strictEqual(none.v.how, 'nothing', '没有可选的成员就不入队命令');
  });

  await ta('sel.apply 入队前先取消活动命令(否则新命令排队不执行)', async () => {
    const c0 = ed3.executor.cancels;
    await callAsync('sel.apply', { items: [{ kind: 'face', id: '1f11' }] });
    assert.ok(ed3.executor.cancels > c0);
  });

  await ta('sel.clear: 执行 Plasticity 的 DeselectAllCommand(click.clear 实测不清空)', async () => {
    const r = await callAsync('sel.clear');
    assert.strictEqual(r.ok, true);
    assert.strictEqual(r.v.how, 'ended');
    assert.strictEqual(s.faceIds.size + s.solidIds.size, 0);
  });

  await ta('R34 材质状态: 定义 + 指派(稳定 id / uuid); 原样不动; 改动走 memento; 找不到如实报', async () => {
    const ed = ed3;
    const s = call('mat.save').v;
    assert.strictEqual(s.v, 1);
    assert.deepStrictEqual(s.materials.map((m) => m.uuid), ['uM1']);
    assert.deepStrictEqual(s.assign, [['idA', 'uM1'], ['idB', 'uM1']], '按稳定 id 排序');
    const r0 = ed.originator.restores;
    assert.deepStrictEqual(call('mat.restore', s).v.applied, ['materials(same)']);
    assert.strictEqual(ed.originator.restores, r0, '没变化不碰 memento(同步返回)');
    // 存档经 Rust(serde_json)转一圈后对象键按字母重排 → 仍应识别为没变化(2026-11-15 自检实测误判)
    const rev = (x) => Array.isArray(x) ? x.map(rev) : (x && typeof x === 'object' ? Object.fromEntries(Object.keys(x).reverse().map((k) => [k, rev(x[k])])) : x);
    assert.deepStrictEqual(call('mat.restore', rev(s)).v.applied, ['materials(same)'], '键序不同不算改动');
    assert.strictEqual(ed.originator.restores, r0);

    // 改颜色 + 新材质指给 idC + 一个不存在的物体
    const B = JSON.parse(JSON.stringify(s));
    B.materials[0].baseColorFactor = [0.9, 0.1, 0.1, 1];
    B.materials.push({ type: 'physical', id: 7, uuid: 'uRed', name: 'Red', baseColorFactor: [1, 0, 0, 1] });
    B.assign = [['idA', 'uM1'], ['idC', 'uRed'], ['idGone', 'uM1'], ['idB', 'uNope']];
    const vp = [...ed.viewports][0];
    const rr0 = vp.renders;
    const r = (await callAsync('mat.restore', B)).v;
    assert.strictEqual(vp.renders, rr0 + 1, '恢复后主动请求重画(实测视口不会自己刷新)');
    assert.deepStrictEqual(r.failed, []);
    assert.ok(r.missing.includes('node:idGone') && r.missing.includes('material:uNope'), JSON.stringify(r));
    assert.deepStrictEqual(ed.matDb.materials[0].baseColorFactor, [0.9, 0.1, 0.1, 1]);
    assert.strictEqual(ed.matDb.counter, 8, 'counter 越过最大 id');
    assert.deepStrictEqual([...ed.nodes.read.node2material.entries()], [[101, 1], [103, 7]]);
    assert.ok(ed.originator.last.geo.untouched, 'memento 其余部分原样');
    assert.ok(ed.originator.last.nodes.materials[1] instanceof Int32Array, '沿用原类型化数组类型');

    // 回到原样
    await callAsync('mat.restore', s);
    assert.deepStrictEqual(call('mat.save').v, s);
    // 参数校验
    assert.strictEqual(call('mat.restore', {}).ok, false);
  });

  await ta('R34d 带贴图的材质: 存档里的纯 JSON 贴图按文件名重新加载成真 Texture; 当前已有的沿用; 无文件名如实报', async () => {
    const ed = ed3;
    const s = call('mat.save').v;
    const gold = { type: 'physical', id: 4, uuid: 'uGold', name: 'Gold foil', baseColorFactor: [1, 0.76, 0.33, 1],
      normalMap: { repeat: 1, texture: new FakeTex('.././img/gold-foil_normal.png').toJSON() },
      roughnessMap: { repeat: 1, texture: new FakeTex('.././img/gold-foil_roughness.png').toJSON() } };
    gold.normalMap.texture.repeat = [2, 3];
    const G = JSON.parse(JSON.stringify(s));
    G.materials.push(gold);
    G.assign = [['idA', 'uGold'], ['idB', 'uM1']];
    const loads0 = ed.materials.textures.loads.length;
    const r = await callAsync('mat.restore', G);
    assert.strictEqual(r.ok, true, JSON.stringify(r));
    assert.deepStrictEqual(r.v.failed, []);
    assert.deepStrictEqual(ed.materials.textures.loads.slice(loads0), ['.././img/gold-foil_normal.png', '.././img/gold-foil_roughness.png']);
    const g = ed.matDb.materials.find((d) => d.uuid === 'uGold');
    assert.ok(g.normalMap.texture instanceof FakeTex && g.normalMap.texture.cloneOf, '加载后 clone 一份(不改共享缓存)');
    assert.deepStrictEqual(g.normalMap.texture.repeat.toArray(), [2, 3], '存档里的贴图参数套回去');
    assert.strictEqual(g.normalMap.repeat, 1, '槽的其它字段保留');
    assert.deepStrictEqual(G.materials[1].normalMap.texture.repeat, [2, 3], '不改调用方的存档对象');

    // 存档 = 当前 + 改颜色: 同 uuid 同文件的贴图沿用当前对象, 不再加载
    const s2 = call('mat.save').v;
    const H = JSON.parse(JSON.stringify(s2));
    H.materials.find((d) => d.uuid === 'uGold').baseColorFactor = [1, 1, 0, 1];
    const live = g.normalMap.texture;
    const loads1 = ed.materials.textures.loads.length;
    assert.strictEqual((await callAsync('mat.restore', H)).ok, true);
    assert.strictEqual(ed.materials.textures.loads.length, loads1, '没有重新加载');
    assert.strictEqual(ed.matDb.materials.find((d) => d.uuid === 'uGold').normalMap.texture, live, '沿用当前 Texture');

    // 没有文件名的贴图(导入图片)恢复不了 → failed 里如实写, 其余照常恢复
    const K = JSON.parse(JSON.stringify(s));
    K.materials.push({ type: 'physical', id: 5, uuid: 'uImg', name: 'Decal', baseColorFactor: [1, 1, 1, 1], map: { repeat: 1, texture: { userData: {}, image: 'x' } } });
    const rk = await callAsync('mat.restore', K);
    assert.strictEqual(rk.ok, true, JSON.stringify(rk));
    assert.deepStrictEqual(rk.v.failed, ['texture:Decal.map']);
    assert.strictEqual(ed.matDb.materials.find((d) => d.uuid === 'uImg').map, undefined);

    await callAsync('mat.restore', s);
    assert.deepStrictEqual(call('mat.save').v, s);

    // restore 中途抛错 → 用恢复前快照整体退回, 不留"物体指向不存在材质"的半套状态; 错误如实上报
    const before = [...ed.nodes.read.node2material.entries()];
    ed.originator.failNext = true;
    const rf = await callAsync('mat.restore', G);
    assert.strictEqual(rf.ok, false, JSON.stringify(rf));
    assert.ok(/undefined is not a function.*rolled back/.test(JSON.stringify(rf)), JSON.stringify(rf));
    assert.deepStrictEqual([...ed.nodes.read.node2material.entries()], before, '指派退回原样');
    assert.deepStrictEqual(call('mat.save').v, s);
  });

  // ---- N2: 命令派发 ----
  // 假工厂(仿 26.2.3 实测): 构造参数 = editor; setter 设参; commit 只在命令里有效(否则不进撤销栈, 这里直接抛);
  // 内部单位米; 布尔用 Parasolid 枚举(15903 = 合并, 其它值抛 PK 错误); 合并吃掉目标物体。
  // 仿 Parasolid 封装(26.2.3 实测形状): 面 GetSurface()/FindMidpoint(), 边 GetCurve()/FindLength()/GetVertices()
  class Plane { GetInfo() { return {}; } }
  class Line { constructor(d) { this.d = d; } GetInfo() { return { origin: v3(0, 0, 0), direction: this.d }; } }
  class Vertex { constructor(p, ident) { this.p = p; this.ident = ident; } GetPoint() { return v3(...this.p); } GetIdentifier() { return this.ident; } }
  // 仿 three.js(只要垫片用到的部分): 显示网格 / 几何 / 属性 / 材质 —— 高亮要用它们的构造函数拷面
  class FakeAttr { constructor(array, itemSize) { this.array = array; this.itemSize = itemSize; this.count = array.length / itemSize; } }
  class FakeGeo { constructor() { this.attributes = {}; this.index = null; } setAttribute(k, v) { this.attributes[k] = v; } dispose() { this.disposed = true; } }
  class FakeMesh { constructor(geometry, material) { this.geometry = geometry; this.material = material; this.renderOrder = 0; } }
  class FakeMat { constructor() { this.color = { hex: 0, set(h) { this.hex = h; } }; this.opacity = 1; } clone() { return new FakeMat(); } dispose() {} }
  function mkFactories(ed) {
    const Command = ed._Command;
    let nextVid = 100, nextTopo = 9000;
    class Solid {
      constructor(mn, mx, extraEdges) {
        this.userData = { versionId: nextVid++ };
        const vid = this.userData.versionId;
        // 盒子 6 个面: 每面 = 包围盒 + 外法向 + 中点
        const fl = [];
        for (const ax of [0, 1, 2]) for (const [s, at] of [[-1, mn[ax]], [1, mx[ax]]]) {
          const lo = mn.slice(), hi = mx.slice();
          lo[ax] = hi[ax] = at;
          const n = [0, 0, 0]; n[ax] = s;
          fl.push({ bb: [...lo, ...hi], normal: n, center: lo.map((v, k) => (v + hi[k]) / 2) });
        }
        // 盒子 12 条边: 每条 = 一个轴向线段的包围盒; 顶点按坐标编号(同一点同一标识)
        const vIdent = new Map();
        const vtx = (p) => { const key = p.join(','); if (!vIdent.has(key)) vIdent.set(key, vIdent.size + 1); return new Vertex(p, vIdent.get(key)); };
        const el = [];
        for (const ax of [0, 1, 2]) {
          const o = [0, 1, 2].filter((k) => k !== ax);
          for (const a of [mn[o[0]], mx[o[0]]]) for (const b of [mn[o[1]], mx[o[1]]]) {
            const lo = [0, 0, 0], hi = [0, 0, 0];
            lo[ax] = mn[ax]; hi[ax] = mx[ax]; lo[o[0]] = hi[o[0]] = a; lo[o[1]] = hi[o[1]] = b;
            el.push({ bb: [...lo, ...hi], lo, hi, ax });
          }
        }
        (extraEdges || []).forEach((e) => el.push({ bb: e, lo: e.slice(0, 3), hi: e.slice(3), ax: 1 }));
        const reg = (kind, list, model) => {
          const ents = list.map(() => nextTopo++);
          const ids = ents.map((n) => vid + kind + n);
          ids.forEach((id, i) => { ed.geo._topo[id] = { versionId: id, parentItem: this, kind, normal: list[i].normal }; ed.geo._models[id] = model(list[i]); });
          return { versionIds: ids, bboxes: Float32Array.from(list.map((x) => x.bb).flat()), ents };
        };
        this.faces = reg('f', fl, (f) => ({
          GetSurface: () => ({ surface: new Plane(), orientation: true }),
          FindMidpoint: () => ({ position: v3(...f.center), normal: v3(...f.normal) }),
        }));
        this.edges = reg('e', el, (e) => ({
          GetCurve: () => { const d = [0, 0, 0]; d[e.ax] = 1; return { curve: new Line(v3(...d)), sense: true }; },
          FindLength: () => ({ length: Math.hypot(e.hi[0] - e.lo[0], e.hi[1] - e.lo[1], e.hi[2] - e.lo[2]) }),
          GetVertices: () => ({ left: vtx(e.lo), right: vtx(e.hi) }),
        }));
        // 显示网格: 每面 2 个外向三角形(右手, 法向朝外), groups = [起点, 数量] × 面
        const pos = [], idx = [], groups = [];
        fl.forEach((f) => {
          const ax = f.normal.findIndex((x) => x !== 0), s = f.normal[ax];
          let [u, v] = [0, 1, 2].filter((q) => q !== ax);
          if ((ax === 1) !== (s < 0)) [u, v] = [v, u];
          const c = (a, b) => { const p = [0, 0, 0]; p[ax] = f.bb[ax]; p[u] = a; p[v] = b; return p; };
          const lo = [f.bb[0], f.bb[1], f.bb[2]], hi = [f.bb[3], f.bb[4], f.bb[5]];
          const base = pos.length / 3;
          [c(lo[u], lo[v]), c(hi[u], lo[v]), c(hi[u], hi[v]), c(lo[u], hi[v])].forEach((p) => pos.push(...p));
          groups.push(idx.length, 6);
          idx.push(base, base + 1, base + 2, base, base + 2, base + 3);
        });
        const geo = new FakeGeo();
        geo.setAttribute('position', new FakeAttr(Float32Array.from(pos), 3));
        geo.index = { array: Uint32Array.from(idx) };
        this.faces.mesh = new FakeMesh(geo, null);
        this.faces.groups = groups;
        // 吸附缓存: 面心(+法向)、边中点(+切向)、顶点(实体 id 自成一套, 与拓扑 id 的数字同源)
        const vEnts = new Map();
        el.forEach((e) => [e.lo, e.hi].forEach((p) => { const key = p.join(','); if (!vEnts.has(key)) vEnts.set(key, { ent: nextTopo++, p }); }));
        const entry = { snapsForItem: {
          faces: { centers: { faceIds: this.faces.ents, positions: Float32Array.from(fl.map((f) => f.center).flat()), normals: Float32Array.from(fl.map((f) => f.normal).flat()) } },
          edges: {
            halves: { edgeIds: this.edges.ents, positions: Float32Array.from(el.map((e) => e.lo.map((x, q) => (x + e.hi[q]) / 2)).flat()),
                      tangents: Float32Array.from(el.map((e) => { const d = [0, 0, 0]; d[e.ax] = 1; return d; }).flat()) },
            circles: { edgeIds: [], centers: new Float32Array(0), axes: new Float32Array(0), quarters: new Float32Array(0), tangents: new Float32Array(0) },
          },
          shellVertices: { vertexIds: [...vEnts.values()].map((x) => x.ent), positions: Float32Array.from([...vEnts.values()].map((x) => x.p).flat()) },
          bbox: { corners: { positions: new Float32Array(0), normals: new Float32Array(0) } },
        } };
        ed.snapScene.canonicals.set('Item:' + vid + '|false', entry);
        ed.snapScene.occurrences.push({ entry, transform: [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1] });
        this.mn = mn; this.mx = mx;
      }
    }
    const add = (s) => { ed.geo.items.push({ view: s, model: {} }); return s; };
    const remove = (s) => { ed.geo.items = ed.geo.items.filter((it) => it.view !== s); };
    class AbstractFactory {
      constructor(editor) { if (!editor || !editor.geo) throw new TypeError("Cannot read properties of undefined (reading 'factoryStateChanged')"); this.editor = editor; }
      resource(cmd) { this.cmd = cmd; return this; }
      guard() { if (!(this.cmd instanceof Command)) throw new Error('commit outside a command'); }
    }
    class CornerBoxFactory extends AbstractFactory {
      set p1(v) { this._p1 = v; } set p2(v) { this._p2 = v; } set height(v) { this._h = v; }
      set targets(v) { this._targets = v; } set operationType(v) { this._op = v; }
      async commit() {
        this.guard();
        const snap = ed.geo.items.slice(); // 撤销栈项 = {name, snapshot}(真机同形; 这里快照是本步之前的物体列表)
        const a = this._p1, b = this._p2, h = this._h;
        let mn = [Math.min(a.x, b.x), Math.min(a.y, b.y), Math.min(a.z, a.z + h)];
        let mx = [Math.max(a.x, b.x), Math.max(a.y, b.y), Math.max(a.z, a.z + h)];
        let extra = [];
        if (this._op !== undefined) {
          if (this._op !== 15903) throw new Error('Operation failed at PK_BODY_boolean_2 with error `PK_ERROR_field_of_wrong_type` (5014)');
          const t = this._targets[0];
          // 合缝边: 工具盒的 x 最大处、目标盒顶面高度, 沿 y 方向
          extra = [[mx[0], t.mn[1], t.mx[2], mx[0], t.mx[1], t.mx[2]]];
          this._targets.forEach((s) => { mn = mn.map((v, k) => Math.min(v, s.mn[k])); mx = mx.map((v, k) => Math.max(v, s.mx[k])); remove(s); });
        }
        ed.history._undoStack.push({ name: 'PlaFactory', snapshot: snap });
        return [add(new Solid(mn, mx, extra))];
      }
    }
    class MultiFilletShellFactory extends AbstractFactory {
      set edges(v) { this._edges = v; } set distance(v) { this._d = v; }
      async commit() {
        this.guard();
        if (this._d > 0.01) throw new Error('Operation failed at PK_BODY_fillet_edges with error `PK_ERROR_failed_to_create_blend`');
        const parent = this._edges[0].parentItem;
        ed.lastFillet = { edges: this._edges.map((e) => e.versionId), distance: this._d };
        const snap = ed.geo.items.slice();
        remove(parent);
        ed.history._undoStack.push({ name: 'PlaFactory', snapshot: snap });
        return [add(new Solid(parent.mn, parent.mx))];
      }
    }
    // 面偏移: 沿面的外法向推出 distance(米), 盒子包围盒随之变化
    class OffsetFaceFactory extends AbstractFactory {
      set faces(v) { this._faces = v; } set distance(v) { this._d = v; }
      async commit() {
        this.guard();
        const f = this._faces[0], parent = f.parentItem, n = f.normal;
        ed.lastOffset = { faces: this._faces.map((x) => x.versionId), distance: this._d };
        const mn = parent.mn.slice(), mx = parent.mx.slice();
        const ax = n.findIndex((x) => x !== 0);
        if (n[ax] > 0) mx[ax] += this._d; else mn[ax] -= this._d;
        const snap = ed.geo.items.slice();
        remove(parent);
        ed.history._undoStack.push({ name: 'PlaFactory', snapshot: snap });
        return [add(new Solid(mn, mx))];
      }
    }
    // 倒圆面改半径: distance = 半径增量(实测语义); 这里只记录
    class RefilletFaceFactory extends AbstractFactory {
      set faces(v) { this._faces = v; } set distance(v) { this._d = v; }
      async commit() { this.guard(); ed.lastRefillet = { faces: this._faces.map((x) => x.versionId), distance: this._d }; return []; }
    }
    // 整组面平移(仿实测): move(Vector3)与 matrix(平移矩阵)都设了才生效, 否则 commit 什么也不做(返回 null, 不进撤销栈)
    class MultiMoveFaceFactory extends AbstractFactory {
      constructor(editor) {
        super(editor);
        this.move = { x: 0, y: 0, z: 0, set(x, y, z) { this.x = x; this.y = y; this.z = z; } };
        this.matrix = { t: null, makeTranslation(x, y, z) { this.t = [x, y, z]; return this; } };
      }
      set faces(v) { this._faces = v; }
      async commit() {
        this.guard();
        const m = this.move;
        if (!(m.x || m.y || m.z) || !this.matrix.t) return null;
        ed.lastMove = { faces: this._faces.map((x) => x.versionId), move: [m.x, m.y, m.z], matrix: this.matrix.t };
        ed.history._undoStack.push({ name: 'PlaFactory', snapshot: ed.geo.items.slice() });
        return [];
      }
    }
    return { CornerBoxFactory, MultiFilletShellFactory, OffsetFaceFactory, RefilletFaceFactory, MultiMoveFaceFactory };
  }

  const L_BRACKET = { ops: [
    { op: 'box', id: 'base', p1: [100, 0, 0], p2: [140, 20, 0], height: 5 },
    { op: 'box', id: 'L', p1: [100, 0, 0], p2: [105, 20, 0], height: 30, bool: 'union', targets: ['$base'] },
    { op: 'fillet', edges: [{ near: [105, 10, 5], of: '$L' }], distance: 3 },
  ] };

  await ta('N2 cmd.exec: 工厂类未挂上 → need-factories(Rust 据此让代理拍快照后重试); 状态接口如实报缺', async () => {
    const r = await callAsync('cmd.exec', L_BRACKET);
    assert.strictEqual(r.ok, false);
    assert.strictEqual(r.err, 'exc:need-factories');
    const f = call('cmd.factories').v;
    assert.strictEqual(f.loaded, false);
    assert.deepStrictEqual(f.missing.sort(), ['fac.cornerBox', 'fac.moveFaces', 'fac.multiFillet', 'fac.offsetFace', 'fac.refillet']);
    assert.deepStrictEqual(f.ops, ['box', 'fillet', 'offsetFace', 'refillet', 'moveFaces']);
  });

  await ta('N2 cmd.exec M-A: L 形支架 = 底板 + 立板(合并) + 内角倒圆; 文档单位 mm → 内部米; 每步一个命令一个撤销步', async () => {
    const ed = ed3;
    ctx.__PLA_FACTORIES__ = mkFactories(ed);
    assert.strictEqual(call('cmd.factories').v.loaded, true);
    assert.deepStrictEqual(call('cmd.factories').v.missing, []);
    const u0 = ed.history._undoStack.length, q0 = ed.enqueued, c0 = ed.executor.cancels;
    const r = await callAsync('cmd.exec', L_BRACKET);
    assert.strictEqual(r.ok, true, JSON.stringify(r));
    const v = r.v;
    assert.strictEqual(v.failed, null, JSON.stringify(v.failed));
    assert.strictEqual(v.units, 'millimeter');
    assert.deepStrictEqual(v.results.map((x) => x.op), ['box', 'box', 'fillet']);
    assert.deepStrictEqual(JSON.parse(JSON.stringify(v.results[0].items[0].bbox)), [[100, 0, 0], [140, 20, 5]], '回读包围盒按文档单位');
    assert.deepStrictEqual(JSON.parse(JSON.stringify(v.results[1].items[0].bbox)), [[100, 0, 0], [140, 20, 30]], '合并后的整体');
    assert.strictEqual(ed.geo.items.length, 1, '底板被合并吃掉, 倒圆替换了 L, 只剩一个实体');
    assert.strictEqual(ed.lastFillet.distance, 0.003, '3 mm → 0.003 m');
    assert.strictEqual(ed.lastFillet.edges.length, 1);
    const seam = ed.geo._topo[ed.lastFillet.edges[0]];
    assert.ok(seam.kind === 'e', '挑中的是边');
    assert.strictEqual(ed.enqueued - q0, 3, '三步三个命令');
    assert.strictEqual(ed.history._undoStack.length - u0, 3, '三个撤销步');
    assert.ok(ed.executor.cancels - c0 >= 3, '每步入队前取消活动命令');
    assert.strictEqual(typeof v.results[2].items[0].id, 'number', '产物带物体 id(可再引用)');
  });

  await ta('N2 cmd.exec: 失败如实报且停在失败步; 已完成的保留; 引用已被吃掉的物体、挑不到边、PK 内核报错、参数错', async () => {
    const ed = ed3;
    // 引用被合并吃掉的底板
    const r1 = (await callAsync('cmd.exec', { ops: [
      { op: 'box', id: 'a', p1: [0, 0, 0], p2: [10, 10, 0], height: 2 },
      { op: 'box', id: 'b', p1: [0, 0, 0], p2: [2, 10, 0], height: 10, bool: 'union', targets: ['$a'] },
      { op: 'fillet', edges: [{ near: [2, 5, 2], of: '$a' }], distance: 1 },
    ] })).v;
    assert.strictEqual(r1.results.length, 2, '前两步保留');
    assert.deepStrictEqual([r1.failed.index, r1.failed.op], [2, 'fillet']);
    assert.ok(/\$a no longer exists/.test(r1.failed.err), r1.failed.err);
    // 按位置挑不到
    const u0 = ed.history._undoStack.length, q0 = ed.enqueued;
    const r2 = (await callAsync('cmd.exec', { ops: [{ op: 'fillet', edges: [{ near: [999, 999, 999] }], distance: 1 }] })).v;
    assert.ok(/edge near \[999,999,999\]: none within 0\.5 \(nearest /.test(r2.failed.err), r2.failed.err);
    assert.strictEqual(ed.enqueued, q0, '选择器出错不产生命令');
    // 内核报错: 命令失败, 不进撤销栈
    const r3 = (await callAsync('cmd.exec', { ops: [{ op: 'fillet', edges: [{ near: [2, 5, 2] }], distance: 50 }] })).v;
    assert.ok(/PK_ERROR_failed_to_create_blend/.test(r3.failed.err), JSON.stringify(r3));
    assert.strictEqual(ed.history._undoStack.length, u0, '失败的步不进撤销栈');
    // 参数错误在执行任何一步之前就报
    const q1 = ed.enqueued;
    const bad = [
      [{ ops: [{ op: 'sphere' }] }, /unknown op \(known: box, fillet, offsetFace, refillet, moveFaces\)/],
      [{ ops: [{ op: 'box', p1: [0, 0, 0], p2: [1, 1, 0] }] }, /missing height/],
      [{ ops: [{ op: 'box', p1: [0, 0, 0], p2: [1, 1, 0], height: 1, colour: 'red' }] }, /unknown param colour/],
      [{ ops: [{ op: 'fillet', edges: ['1e1'], distance: 1, bool: 'union' }] }, /bool not supported/],
      [{ ops: [{ op: 'box', p1: [0, 0, 0], p2: [1, 1, 0], height: 1, bool: 'xor' }] }, /bool must be one of union, subtract, intersect/],
      [{ ops: [] }, /needs \{ops/],
      [{ units: 'furlong', ops: [{ op: 'box', p1: [0, 0, 0], p2: [1, 1, 0], height: 1 }] }, /unknown unit: furlong/],
    ];
    for (const [args, re] of bad) {
      const r = await callAsync('cmd.exec', args);
      assert.strictEqual(r.ok, false, JSON.stringify(args));
      assert.ok(re.test(r.err), r.err);
    }
    assert.strictEqual(ed.enqueued, q1, '参数错不入队任何命令');
  });

  await ta('N2 cmd.exec: units 可覆盖文档单位; 物体可按 id 引用', async () => {
    const ed = ed3;
    const r = (await callAsync('cmd.exec', { units: 'meter', ops: [{ op: 'box', id: 'm', p1: [0, 0, 0], p2: [0.01, 0.01, 0], height: 0.01 }] })).v;
    assert.strictEqual(r.units, 'meter');
    assert.deepStrictEqual(JSON.parse(JSON.stringify(r.results[0].items[0].bbox)), [[0, 0, 0], [0.01, 0.01, 0.01]]);
    const id = r.results[0].items[0].id;
    const r2 = (await callAsync('cmd.exec', { ops: [{ op: 'box', p1: [0, 0, 0], p2: [5, 10, 0], height: 20, bool: 'union', targets: [id] }] })).v;
    assert.strictEqual(r2.failed, null, JSON.stringify(r2.failed));
    assert.ok(!ed.geo.items.some((it) => it.view.userData.versionId === id), '按 id 引用的目标被合并');
  });

  await ta('A1 geo.query: 实体概览 / 面签名(类型+中点+法向) / 边签名(类型+长度+端点) / 顶点去重; 过滤与单位', async () => {
    const ed = ed3;
    const r = (await callAsync('cmd.exec', { ops: [{ op: 'box', p1: [0, 0, 0], p2: [10, 20, 0], height: 5 }] })).v;
    const id = r.results[0].items[0].id;
    // 概览: 默认只列实体
    const s = call('geo.query').v;
    assert.strictEqual(s.units, 'millimeter');
    assert.ok(s.solids.length >= 2 && !s.faces && !s.edges, '默认 detail=solids');
    const me = s.solids.find((x) => x.id === id);
    assert.deepStrictEqual(JSON.parse(JSON.stringify(me.bbox)), [[0, 0, 0], [10, 20, 5]]);
    assert.deepStrictEqual([me.faces, me.edges], [6, 12]);
    // 面: of 限定物体; 法向过滤挑出顶面
    const f = call('geo.query', { detail: 'faces', of: [id] }).v;
    assert.strictEqual(f.solids.length, 1);
    assert.strictEqual(f.faces.length, 6);
    assert.ok(f.faces.every((x) => x.surface === 'Plane' && x.solid === id && /^\d+f\d+$/.test(x.id)), JSON.stringify(f.faces[0]));
    const top = call('geo.query', { detail: 'faces', of: [id], normal: [0, 0, 2] }).v.faces;
    assert.strictEqual(top.length, 1, '法向 +Z 只有顶面(法向会先归一化)');
    assert.deepStrictEqual(JSON.parse(JSON.stringify(top[0].center)), [5, 10, 5]);
    assert.deepStrictEqual(JSON.parse(JSON.stringify(top[0].normal)), [0, 0, 1]);
    assert.strictEqual(call('geo.query', { detail: 'faces', of: [id], near: [5, 10, 5] }).v.faces.length, 1, '按包围盒中心就近');
    // 返回的面 id 可直接给 cmd.exec 的选择器用(同一套拓扑 id)
    assert.strictEqual(call('geo.query', { detail: 'faces', of: [id], surface: 'Cylinder' }).v.faces.length, 0);
    // 边 + 顶点
    const a = call('geo.query', { detail: 'all', of: [id] }).v;
    assert.strictEqual(a.edges.length, 12);
    assert.deepStrictEqual(a.edges.map((e) => e.length).sort((x, y) => x - y), [5, 5, 5, 5, 10, 10, 10, 10, 20, 20, 20, 20]);
    assert.ok(a.edges.every((e) => e.curve === 'Line' && e.start && e.end && e.direction), JSON.stringify(a.edges[0]));
    assert.strictEqual(a.vertices.length, 8, '顶点按标识去重');
    assert.ok(a.vertices.every((v) => new RegExp('^' + id + 'v\\d+$').test(v.id)));
    // 单位覆盖: 米
    const m = call('geo.query', { of: [id], units: 'meter' }).v;
    assert.deepStrictEqual(JSON.parse(JSON.stringify(m.solids[0].bbox)), [[0, 0, 0], [0.01, 0.02, 0.005]]);
    // 截断如实报
    const t = call('geo.query', { detail: 'faces', limit: 2 }).v;
    assert.strictEqual(t.faces.length, 2);
    assert.ok(t.truncated.faces > 0, JSON.stringify(t.truncated));
    // 参数错
    assert.ok(/detail must be/.test(call('geo.query', { detail: 'x' }).err));
    assert.ok(/solid not found/.test(call('geo.query', { of: [99999] }).err));
    assert.ok(/normal must be/.test(call('geo.query', { detail: 'faces', normal: [0, 0] }).err));
  });

  await ta('A2 geo.snaps: 读吸附缓存(面心+法向 / 边中点+切向 / 顶点实体 id), 拓扑 id 与 geo.query 一致; 就近过滤; 缺缓存如实报', async () => {
    const ed = ed3;
    const r = (await callAsync('cmd.exec', { ops: [{ op: 'box', p1: [0, 0, 0], p2: [10, 20, 0], height: 5 }] })).v;
    const id = r.results[0].items[0].id;
    const s = call('geo.snaps', { of: [id] }).v;
    const count = (k) => s.points.filter((p) => p.kind === k).length;
    assert.deepStrictEqual([count('faceCenter'), count('edgeMid'), count('vertex')], [6, 12, 8]);
    const top = call('geo.snaps', { of: [id], kinds: ['faceCenter'], near: [5, 10, 5] }).v.points;
    assert.strictEqual(top.length, 1);
    assert.deepStrictEqual(JSON.parse(JSON.stringify(top[0])).normal, [0, 0, 1]);
    const qTop = call('geo.query', { detail: 'faces', of: [id], normal: [0, 0, 1] }).v.faces[0];
    assert.strictEqual(top[0].id, qTop.id, '吸附点的面 id 与 geo.query 同一套');
    // geo.query 的顶点改用吸附数据里的实体 id
    const qv = call('geo.query', { detail: 'all', of: [id] }).v.vertices.map((v) => v.id).sort();
    assert.deepStrictEqual(qv, s.points.filter((p) => p.kind === 'vertex').map((p) => p.id).sort());
    // 缺缓存
    const key = [...ed.snapScene.canonicals.keys()].find((k) => k.startsWith('Item:' + id + '|'));
    const saved = ed.snapScene.canonicals.get(key);
    ed.snapScene.canonicals.delete(key);
    const miss = call('geo.snaps', { of: [id] }).v;
    assert.deepStrictEqual([miss.points.length, miss.noSnapData], [0, [id]]);
    assert.strictEqual(call('geo.query', { detail: 'all', of: [id] }).v.vertices.length, 8, '没缓存时顶点退回边端点');
    ed.snapScene.canonicals.set(key, saved);
    assert.ok(/unknown snap kind/.test(call('geo.snaps', { kinds: ['nope'] }).err));
  });

  await ta('A2 geo.measure: 距离(点/面/边/顶点; 平行平面给法向距离=壁厚)、角度、面积/体积/质心(网格)', async () => {
    const r = (await callAsync('cmd.exec', { ops: [{ op: 'box', p1: [0, 0, 0], p2: [10, 20, 0], height: 5 }] })).v;
    const id = r.results[0].items[0].id;
    const f = (n) => call('geo.query', { detail: 'faces', of: [id], normal: n }).v.faces[0].id;
    const top = f([0, 0, 1]), bot = f([0, 0, -1]), side = f([1, 0, 0]);
    const d = call('geo.measure', { op: 'distance', a: top, b: bot }).v;
    assert.strictEqual(d.distance, 5);
    assert.strictEqual(d.normalDistance, 5, '平行平面 → 法向距离(壁厚)');
    const d2 = call('geo.measure', { op: 'distance', a: top, b: side }).v;
    assert.strictEqual(d2.normalDistance, undefined, '不平行不给');
    assert.strictEqual(call('geo.measure', { op: 'distance', a: [0, 0, 0], b: [3, 4, 0] }).v.distance, 5);
    const vs = call('geo.snaps', { of: [id], kinds: ['vertex'] }).v.points;
    const v0 = vs.find((p) => p.pos.join() === '0,0,0'), v1 = vs.find((p) => p.pos.join() === '10,20,5');
    assert.strictEqual(call('geo.measure', { op: 'distance', a: v0.id, b: v1.id }).v.distance, Number(Math.hypot(10, 20, 5).toPrecision(7)));
    assert.strictEqual(call('geo.measure', { op: 'angle', a: top, b: side }).v.angle, 90);
    assert.strictEqual(call('geo.measure', { op: 'angle', a: top, b: bot }).v.angle, 180);
    const ex = call('geo.query', { detail: 'edges', of: [id] }).v.edges.find((e) => e.length === 10);
    assert.strictEqual(call('geo.measure', { op: 'angle', a: ex.id, b: [1, 0, 0] }).v.angle, 0, '边取直线方向');
    const m = call('geo.measure', { op: 'mass', of: [id], faces: true }).v.solids[0];
    assert.deepStrictEqual([m.volume, m.area, m.method], [1000, 700, 'mesh']);
    assert.deepStrictEqual(JSON.parse(JSON.stringify(m.centroid)), [5, 10, 2.5]);
    assert.deepStrictEqual(m.faces.map((x) => x.area).sort((a, b) => a - b), [50, 50, 100, 100, 200, 200]);
    assert.strictEqual(call('geo.measure', { op: 'mass', of: [id], units: 'meter' }).v.solids[0].volume, 1e-6);
    assert.ok(/op must be/.test(call('geo.measure', { op: 'x' }).err));
    assert.ok(/reference must be/.test(call('geo.measure', { op: 'distance', a: 'abc', b: [0, 0, 0] }).err));
  });

  await ta('A5 cam.view: 临时切正交 → 按视角朝向 + 包围盒投影算缩放对准物体 → 截图 → 还原相机与投影', async () => {
    const ed = ed3, vp = [...ed.viewports][0];
    const r = (await callAsync('cmd.exec', { ops: [{ op: 'box', p1: [0, 0, 0], p2: [10, 20, 0], height: 5 }] })).v;
    const id = r.results[0].items[0].id;
    vp.camMode = 'perspective';
    const q0 = [vp.camera.quaternion.x, vp.camera.quaternion.y, vp.camera.quaternion.z, vp.camera.quaternion.w];
    const z0 = vp.orbitControls.zoom, t0 = vp.renders2.length;
    const v = call('cam.view', { view: 'front', of: [id] });
    assert.strictEqual(v.ok, true, JSON.stringify(v));
    assert.strictEqual(v.v.data, 'UE5H');
    assert.deepStrictEqual(JSON.parse(JSON.stringify(v.v.target)), [5, 10, 2.5], '对准包围盒中心(文档单位)');
    const at = vp.renders2[vp.renders2.length - 1];
    assert.ok(vp.renders2.length > t0);
    assert.strictEqual(at.mode, 'orthographic', '截图时是正交');
    const s = Math.SQRT1_2;
    at.q.forEach((x, i) => assert.ok(Math.abs(x - [s, 0, 0, s][i]) < 1e-9, '前视: 绕 X 转 90° ' + JSON.stringify(at.q)));
    // 半宽 5 mm / aspect 1.5 = 3.33 mm > 半高 2.5 mm → zoom = 3 / (0.00333 × 1.15)
    assert.ok(Math.abs(at.zoom - 3 / ((0.005 / 1.5) * 1.15)) < 1e-6, String(at.zoom));
    assert.deepStrictEqual(at.target.map((x) => Math.round(x * 1e6) / 1e6), [0.005, 0.01, 0.0025]);
    // 还原
    assert.strictEqual(vp.camMode, 'perspective', '投影切回去');
    assert.deepStrictEqual([vp.camera.quaternion.x, vp.camera.quaternion.y, vp.camera.quaternion.z, vp.camera.quaternion.w], q0, '相机还原');
    assert.strictEqual(vp.orbitControls.zoom, z0);
    // 等轴 / 顶视: 朝向正确(相机 z 轴 = 视角方向)
    call('cam.view', { view: 'top', of: [id] });
    const tq = vp.renders2[vp.renders2.length - 1].q;
    assert.ok(Math.abs(tq[3]) > 0.999 || Math.abs(tq[2]) > 0.999, '顶视: 相机朝 -Z 看, 无旋转或绕 Z 180° ' + JSON.stringify(tq));
    assert.strictEqual(call('cam.view', { view: 'iso' }).ok, true);
    assert.ok(/view must be one of/.test(call('cam.view', { view: 'side' }).err));
    vp.camMode = 'orthographic';
  });

  await ta('N3 view.highlight: 拷面三角形成叠加网格(角色 → 颜色, 透过实体可见)加进辅助场景; 每次替换上次; 空 = 清除', async () => {
    const ed = ed3;
    ed.helpers = { items: [], add(m) { this.items.push(m); }, remove(m) { this.items = this.items.filter((x) => x !== m); } };
    ed.gizmos = { yellow: { mesh: new FakeMat() } };
    const r = (await callAsync('cmd.exec', { ops: [{ op: 'box', p1: [0, 0, 0], p2: [10, 20, 0], height: 5 }] })).v;
    const id = r.results[0].items[0].id;
    const top = call('geo.query', { detail: 'faces', of: [id], normal: [0, 0, 1] }).v.faces[0].id;
    const bot = call('geo.query', { detail: 'faces', of: [id], normal: [0, 0, -1] }).v.faces[0].id;
    const h = call('view.highlight', { faces: [{ id: bot, role: 'primary' }, { id: top, role: 'candidate' }, { id: '99999f1' }] });
    assert.strictEqual(h.ok, true, JSON.stringify(h));
    assert.deepStrictEqual(h.v.shown, [bot, top]);
    assert.ok(/99999f1/.test(h.v.failed[0]), '查不到的面如实报');
    assert.strictEqual(ed.helpers.items.length, 2);
    const m0 = ed.helpers.items[0];
    assert.strictEqual(m0.geometry.attributes.position.array.length, 6 * 3, '一个面 2 个三角形 = 6 个顶点');
    assert.ok(m0.material.color.hex === 0xff8c1a && m0.material.depthTest === false, '主色 + 透过实体可见');
    assert.ok(m0.renderOrder > 0);
    // 再次调用替换上次; 空 = 清除
    call('view.highlight', { faces: [{ id: top }] });
    assert.strictEqual(ed.helpers.items.length, 1);
    assert.strictEqual(m0.geometry.disposed, true, '旧网格释放');
    call('view.highlight', { faces: [] });
    assert.strictEqual(ed.helpers.items.length, 0);
    // dim.face 的候选列表 + 指定对面
    const d = call('dim.face', { face: top }).v;
    assert.deepStrictEqual(d.candidates.map((c) => [c.id, c.value, c.overlap]), [[bot, 5, true]]);
    assert.strictEqual(call('dim.face', { face: top, opposite: bot }).v.value, 5);
    assert.ok(/not an opposite parallel face/.test(call('dim.face', { face: top, opposite: top }).err));
  });

  await ta('N3c dim.distance: 参考面(第一个) → 动的面组; 平面锚 → 法向距离 + 方向; moveFaces 同时设 move 与 matrix; 参数错如实报', async () => {
    const ed = ed3;
    const r = (await callAsync('cmd.exec', { ops: [{ op: 'box', p1: [0, 0, 0], p2: [10, 20, 0], height: 5 }] })).v;
    const id = r.results[0].items[0].id;
    const f = (n) => call('geo.query', { detail: 'faces', of: [id], normal: n }).v.faces[0].id;
    const top = f([0, 0, 1]), bot = f([0, 0, -1]), side = f([1, 0, 0]);
    const d = call('dim.distance', { reference: bot, moving: [top] }).v;
    assert.deepStrictEqual([d.kind, d.value, d.anchor, d.anchorFace, d.op], ['distance', 5, 'plane', top, 'moveFaces']);
    assert.deepStrictEqual(JSON.parse(JSON.stringify(d.direction)), [0, 0, 1], '从参考面指向动的面');
    // 不平行的面 → 用面中点(近似, 如实标出)
    const c = call('dim.distance', { reference: bot, moving: [side] }).v;
    assert.deepStrictEqual([c.anchor, c.value], ['center', 2.5]);
    // 改值: 整组沿 direction 平移 (新 − 旧)
    const ex = (await callAsync('cmd.exec', { atomic: true, ops: [{ op: 'moveFaces', faces: [top], move: d.direction.map((x) => x * (8 - d.value)) }] })).v;
    assert.strictEqual(ex.failed, null, JSON.stringify(ex.failed));
    assert.deepStrictEqual(ed.lastMove.move.map((x) => Math.round(x * 1e9) / 1e9), [0, 0, 0.003], '3 mm → 0.003 m');
    assert.deepStrictEqual(ed.lastMove.matrix.map((x) => Math.round(x * 1e9) / 1e9), [0, 0, 0.003], '平移矩阵同时设');
    // 参数错
    assert.ok(/needs \{reference, moving/.test(call('dim.distance', { reference: bot }).err));
    assert.ok(/also in moving/.test(call('dim.distance', { reference: bot, moving: [bot] }).err));
    assert.ok(/face not found/.test(call('dim.distance', { reference: bot, moving: ['99999f1'] }).err));
    assert.ok(/move must be \[dx,dy,dz\]/.test((await callAsync('cmd.exec', { ops: [{ op: 'moveFaces', faces: [top], move: [1, 2] }] })).v.failed.err));
  });

  await ta('N3 dim.face: 平面 → 厚度(背对平行面) + 偏移系数; 来自选择或显式 id; offsetFace 改厚度后再读 = 新值', async () => {
    const ed = ed3;
    const r = (await callAsync('cmd.exec', { ops: [{ op: 'box', p1: [0, 0, 0], p2: [10, 20, 0], height: 5 }] })).v;
    const id = r.results[0].items[0].id;
    const top = call('geo.query', { detail: 'faces', of: [id], normal: [0, 0, 1] }).v.faces[0].id;
    const bot = call('geo.query', { detail: 'faces', of: [id], normal: [0, 0, -1] }).v.faces[0].id;
    const d = call('dim.face', { face: top }).v;
    assert.deepStrictEqual([d.kind, d.value, d.opposite, d.op, d.offsetSign, d.units], ['thickness', 5, bot, 'offsetFace', 1, 'millimeter']);
    const side = call('geo.query', { detail: 'faces', of: [id], normal: [1, 0, 0] }).v.faces[0].id;
    assert.strictEqual(call('dim.face', { face: side }).v.value, 10, 'x 向宽度');
    // 来自选择: 只选中一个面
    const s = ed.selection.selected, saved = new Set(s.faceIds);
    s.faceIds.clear(); s.faceIds.add(top);
    assert.strictEqual(call('dim.face').v.face, top);
    s.faceIds.clear();
    assert.ok(/select exactly one face/.test(call('dim.face').err));
    saved.forEach((x) => s.faceIds.add(x));
    // 改值: offsetFace(选中面, (新 − 旧) × offsetSign)
    const ex = (await callAsync('cmd.exec', { atomic: true, ops: [{ op: 'offsetFace', faces: [top], distance: (8 - d.value) * d.offsetSign }] })).v;
    assert.strictEqual(ex.failed, null, JSON.stringify(ex.failed));
    assert.ok(ed.lastOffset && Math.abs(ed.lastOffset.distance - 0.003) < 1e-12, '3 mm → 0.003 m');
    assert.ok(/face not found/.test(call('dim.face', { face: '99999f1' }).err));
  });

  await ta('A3 cmd.exec atomic: 某步失败 → 撤回本次全部已完成步骤(edit:undo), 文档回到执行前; 窗口不可见直接拒绝', async () => {
    const ed = ed3;
    const n0 = ed.geo.items.length, u0 = ed.history._undoStack.length, d0 = ed.undos || 0;
    const r = (await callAsync('cmd.exec', { atomic: true, ops: [
      { op: 'box', id: 'a', p1: [500, 0, 0], p2: [520, 10, 0], height: 2 },
      { op: 'box', id: 'b', p1: [500, 0, 0], p2: [502, 10, 0], height: 10, bool: 'union', targets: ['$a'] },
      { op: 'fillet', edges: [{ near: [502, 5, 2], of: '$b' }], distance: 50 },
    ] })).v;
    assert.deepStrictEqual([r.failed.index, r.failed.op], [2, 'fillet']);
    assert.ok(/PK_ERROR/.test(r.failed.err), r.failed.err);
    assert.strictEqual(r.rolledBack, 2, '两步都撤回');
    assert.deepStrictEqual(r.results, [], '撤回后不再报"已完成"的产物');
    assert.strictEqual(ed.history._undoStack.length, u0, '撤销栈回到起点');
    assert.strictEqual(ed.geo.items.length, n0, '物体回到执行前');
    assert.strictEqual((ed.undos || 0) - d0, 2, '走 edit:undo 两次');
    // 成功的 atomic 不撤任何东西
    const ok = (await callAsync('cmd.exec', { atomic: true, ops: [{ op: 'box', p1: [600, 0, 0], p2: [601, 1, 0], height: 1 }] })).v;
    assert.strictEqual(ok.failed, null);
    assert.strictEqual(ok.rolledBack, undefined);
    assert.strictEqual(ed.history._undoStack.length, u0 + 1);
    // 窗口不可见: 撤不了(memento) → 一开始就拒绝, 不执行任何一步
    ctx.document.visibilityState = 'hidden';
    const q = ed.enqueued;
    const h = await callAsync('cmd.exec', { atomic: true, ops: [{ op: 'box', p1: [0, 0, 0], p2: [1, 1, 0], height: 1 }] });
    delete ctx.document.visibilityState;
    assert.ok(/atomic needs a visible window/.test(h.err), h.err);
    assert.strictEqual(ed.enqueued, q);
  });

  await ta('A3 hist.begin / rollback / commit: 跨多次调用的事务; begin 以下的撤销栈被动过 → 拒绝回滚', async () => {
    const ed = ed3;
    const n0 = ed.geo.items.length, u0 = ed.history._undoStack.length;
    assert.deepStrictEqual(call('hist.begin').v, { depth: u0 });
    await callAsync('cmd.exec', { ops: [{ op: 'box', p1: [700, 0, 0], p2: [701, 1, 0], height: 1 }] });
    await callAsync('cmd.exec', { ops: [{ op: 'box', p1: [710, 0, 0], p2: [711, 1, 0], height: 1 }] });
    assert.strictEqual(ed.geo.items.length, n0 + 2);
    const r = await callAsync('hist.rollback');
    assert.strictEqual(r.ok, true, JSON.stringify(r));
    assert.deepStrictEqual(r.v, { undone: 2, depth: u0 });
    assert.strictEqual(ed.geo.items.length, n0);
    assert.ok(/no transaction/.test((await callAsync('hist.rollback')).err), '回滚后事务结束');
    // commit
    call('hist.begin');
    await callAsync('cmd.exec', { ops: [{ op: 'box', p1: [720, 0, 0], p2: [721, 1, 0], height: 1 }] });
    assert.deepStrictEqual(call('hist.commit').v, { committed: true, steps: 1 });
    assert.ok(/no transaction/.test((await callAsync('hist.rollback')).err));
    // 用户在事务期间撤到 begin 之前 → 拒绝, 不乱撤
    call('hist.begin');
    ed._undo(); ed._undo();
    const u1 = ed.history._undoStack.length;
    const bad = await callAsync('hist.rollback');
    assert.ok(/below the transaction start/.test(bad.err), bad.err);
    assert.strictEqual(ed.history._undoStack.length, u1, '一步都没撤');
    // 栈深度够、但 begin 以下的某项被换掉(撤了再做了别的)→ 同样拒绝
    call('hist.begin');
    ed._undo();
    await callAsync('cmd.exec', { ops: [{ op: 'box', p1: [730, 0, 0], p2: [731, 1, 0], height: 1 }] });
    await callAsync('cmd.exec', { ops: [{ op: 'box', p1: [740, 0, 0], p2: [741, 1, 0], height: 1 }] });
    assert.ok(/history changed below the transaction start/.test((await callAsync('hist.rollback')).err));
    call('hist.commit');
  });

  await ta('batch 里混进异步接口如实报错, 不吞', async () => {
    const r = JSON.parse(run(`window.__PLA__.batch([["sel.clear",{}],["hist.state",{}]])`));
    assert.strictEqual(r['sel.clear'].ok, false);
    assert.strictEqual(r['hist.state'].ok, true);
  });

  console.log(`shim contract: ${n} passed`);
})().catch((e) => { console.error(e); process.exit(1); });

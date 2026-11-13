// 实验(demo1): 整个选择集在一个命令的 execute 里完成(首个替换、其余追加; 选中实体没选中面的再点一次下钻) → 撤销栈只 +1?
(function () {
  var ed = window.__PLA_EDITOR__; var X = window.__X;
  var Base = Object.getPrototypeOf(ed.commands.DeselectAllCommand.prototype).constructor;
  class PlaApplySelection extends Base {
    constructor(editor, faces) { super(editor); this.faces = faces; this.report = null; }
    async execute() {
      var cs = this.editor.changeSelection; var sel = this.editor.selection.selected;
      var drills = 0;
      var has = function (vid) { return Array.from(sel.faceIds).map(String).indexOf(vid) >= 0; };
      this.faces.forEach(function (face, i) {
        var hit = X.hitFor(face);
        cs.onClick([hit], i === 0 ? 0 : 1, 0);
        if (!has(String(face.versionId))) { drills++; cs.onClick([hit], 1, 0); }
      });
      this.report = { drills: drills };
    }
  }
  var vids = [];
  ed.geo.items.forEach(function (it) { var fg = it.view.faces; if (!fg) return; for (var i = 0; i < fg.faceIds.length && i < 6 && vids.length < 40; i += 2) vids.push(fg.parentId + 'f' + fg.faceIds[i]); });
  var faces = vids.map(function (v) { return X.faceById(v).face; });
  try { ed.executor.cancelActiveCommand(); } catch (e) {}
  window.__PLA__.call('sel.clear'); X.take();
  var undo0 = X.state().undo;
  var S = ed.selection.signals;
  var t0 = performance.now();
  var cmd = new PlaApplySelection(ed, faces);
  var done = new Promise(function (res) {
    var f = function (c) { if (c === cmd) { S.commandEnded.remove(f); res('ended'); } };
    S.commandEnded.add(f); setTimeout(function () { S.commandEnded.remove(f); res('timeout'); }, 5000);
  });
  ed.enqueue(cmd);
  return done.then(function (how) {
    var st = X.state();
    var sel = st.faces;
    return JSON.stringify({ how: how, ms: Math.round(performance.now() - t0), targets: vids.length, selected: sel.length,
      missing: vids.filter(function (v) { return sel.indexOf(v) < 0; }).length, extra: sel.filter(function (v) { return vids.indexOf(v) < 0; }).length,
      undoDelta: st.undo - undo0, report: cmd.report, solidsSelected: st.solids, log: X.take().slice(0, 8) }, null, 1);
  });
})()

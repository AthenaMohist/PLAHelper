# D0: 用 FreeCAD 界面模块导出整页 SVG / PDF(带图框、标题栏、剖面线、尺寸)。
# 要在 freecad.exe 里跑, 但设 QT_QPA_PLATFORM=offscreen, 不出窗口:
#   $env:QT_QPA_PLATFORM='offscreen'; $env:D0_OUT=<d0_freecad.py 的输出目录>; freecad.exe d0_export_gui.py
import os
import time
import traceback

import FreeCAD as App
import FreeCADGui as Gui

OUT = os.environ.get("D0_OUT") or os.path.join(os.path.dirname(os.path.abspath(__file__)), "out")
log = open(os.path.join(OUT, "gui_export.log"), "w", encoding="utf-8")
try:
    t0 = time.time()
    doc = App.openDocument(os.path.join(OUT, "d0.FCStd"))
    doc.recompute()
    import TechDrawGui

    page = doc.getObject("Page")
    # 界面侧(ViewProvider)属性: 圆心标记、导出时也画
    for o in doc.Objects:
        vo = getattr(o, "ViewObject", None)
        if vo is not None and hasattr(vo, "ArcCenterMarks"):
            vo.ArcCenterMarks = True
            if hasattr(vo, "CenterMarks"):
                vo.CenterMarks = True
    # 视图图元只在页面打开(MDI 页)时才建 → 先打开页面, 让界面跑几轮事件
    page.ViewObject.doubleClicked()
    for _ in range(20):
        Gui.updateGui()
        time.sleep(0.05)
    Gui.Selection.clearSelection()
    TechDrawGui.exportPageAsSvg(page, os.path.join(OUT, "d0.svg"))
    TechDrawGui.exportPageAsPdf(page, os.path.join(OUT, "d0.pdf"))
    log.write("ok %.2fs\n" % (time.time() - t0))
except Exception:
    log.write(traceback.format_exc())
finally:
    log.close()
    os._exit(0)

# 在 freecad.exe(离屏)里跑编译器, 出错把堆栈写进 <out>/compile_error.txt, 无论成败都退出进程(不然界面会挂着)。
# PowerShell:
#   $env:QT_QPA_PLATFORM='offscreen'; $env:QT_QPA_FONTDIR='C:\Windows\Fonts'; $env:PLA_DRAW_SPEC=<spec.json>
#   & "<FreeCAD>\bin\freecad.exe" run_offscreen.py
import os
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
code = 0
try:
    src = open(os.path.join(HERE, "compile_freecad.py"), encoding="utf-8").read()
    exec(compile(src, os.path.join(HERE, "compile_freecad.py"), "exec"), {"__name__": "__compile__", "__file__": os.path.join(HERE, "compile_freecad.py")})
except BaseException:
    code = 1
    try:
        import json

        spec = os.environ.get("PLA_DRAW_SPEC", "")
        out = json.load(open(spec, encoding="utf-8-sig")).get("out") if spec else None
        out = out or os.path.join(HERE, "out")
        os.makedirs(out, exist_ok=True)
        open(os.path.join(out, "compile_error.txt"), "w", encoding="utf-8").write(traceback.format_exc())
    except BaseException:
        pass
finally:
    # 先正常关文档(删掉它在缓存里的恢复目录), 再硬退出; 否则下次启动会弹"恢复文档"对话框, 离屏模式下永远等着
    try:
        import FreeCAD as App

        for name in list(App.listDocuments()):
            App.closeDocument(name)
    except BaseException:
        pass
    os._exit(code)

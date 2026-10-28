// 在 renderer 侧执行的 editor 定位与挂载 (由代理经 __PLA_PROXY_SEND 调 Runtime.evaluate 注入)
// 返回: "OK" 或 "ERR: <原因>"
(function () {
  if (typeof window === 'undefined') return 'ERR: no window';
  if (window.__PLA_EDITOR__) return 'OK: already';
  // 26.2.3: editor 在闭包内, 通过堆快照定位 (由 main 侧驱动), 此处仅兜底:
  // 页面 world 无法直接拿到; 依赖代理的堆定位流程。
  return 'ERR: need-heap-locate';
})()

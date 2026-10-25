/* PLA-CDP patch v6 (ASCII-only on purpose: this text is merged into Plasticity's index.js).
   Opens the Chrome DevTools port 9333 so PLAhelper can talk to Plasticity.
   - neutralizes the app's own removeSwitch() of both debugging switches
   - appends remote-debugging-port=9333 unless one is already supplied
   - PLA_CDP_INSTANCE=1 -> multi-instance (separate profile)
   - writes startup/exit/error log to ~/.pla-cdp-patch.log
   No application logic is modified. */
(()=>{try{
const fs=require("fs"),path=require("path"),os=require("os");
const LOG=path.join(os.homedir(),".pla-cdp-patch.log");
const lg=(m)=>{try{fs.appendFileSync(LOG,new Date().toISOString().slice(11,23)+" "+m+"\n")}catch(e){}};
lg("patch v6 enter pid="+process.pid);
process.on("exit",(c)=>lg("EXIT code="+c));
process.on("uncaughtException",(e)=>lg("UNCAUGHT: "+(e&&e.stack||e)));
process.on("unhandledRejection",(e)=>lg("UNHANDLED: "+String(e)));
const{app}=require("electron");
if(app&&app.commandLine){
  const o=app.commandLine.removeSwitch.bind(app.commandLine);
  const has=app.commandLine.hasSwitch.bind(app.commandLine);
  const external=has("remote-debugging-port");
  app.commandLine.removeSwitch=(s)=>{if(s==="remote-debugging-port"||s==="remote-debugging-pipe")return;o(s)};
  if(!external){app.commandLine.appendSwitch("remote-debugging-port",process.env.PLA_CDP_PORT||"9333")}
  if(process.env.PLA_REVERSE){app.commandLine.appendSwitch("js-flags","--allow-natives-syntax --no-lazy");lg("PLA_REVERSE on")}
  if(has("inspect")){process.argv=[process.argv[0]];lg("argv sanitized")}
  lg("patch v6 done external="+external)
}
if(process.env.PLA_CDP_INSTANCE){app.requestSingleInstanceLock=()=>true;app.setPath("userData",process.env.PLA_CDP_PROFILE||path.join(os.homedir(),".plasticity-cdp-profile"))}
app.on("window-all-closed",()=>lg("window-all-closed"));
app.on("quit",(e,c)=>lg("app.quit code="+c));
app.on("render-process-gone",(e,d)=>lg("render-gone "+JSON.stringify(d&&d.reason)))
}catch(e){try{const f=require("fs");f.appendFileSync(require("path").join(require("os").homedir(),".pla-cdp-patch.log"),"PATCH-ERR "+(e&&e.stack||e)+"\n")}catch(e2){}}})();

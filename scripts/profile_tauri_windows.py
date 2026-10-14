"""Measure our own native pet before/after on-demand panel cycles. No clipboard capture."""
import argparse
import ctypes
from ctypes import wintypes
import json
import os
import pathlib
import queue
import subprocess
import threading
import time

parser=argparse.ArgumentParser()
parser.add_argument("--binary",required=True)
parser.add_argument("--output",required=True)
parser.add_argument("--skin")
parser.add_argument("--idle-only", action="store_true", help="Measure a legacy/native baseline without panel control")
parser.add_argument("--seconds", type=int, default=12, help="Seconds sampled in each phase")
options=parser.parse_args()
if not 1 <= options.seconds <= 300:
    parser.error("--seconds must be between 1 and 300")
root=pathlib.Path(__file__).resolve().parents[1]
output=pathlib.Path(options.output).resolve();output.mkdir(parents=True,exist_ok=True)
data=output/"data";data.mkdir(exist_ok=True)
(data/"config.json").write_text(json.dumps({"level":"Calm","clipboard_enabled":False,"roaming":False,"allow_hide_icons":False,"allow_cursor_chase":False}),encoding="utf-8")
arguments=[str(pathlib.Path(options.binary).resolve()),"--data-dir",str(data),"--exit-after",str(options.seconds + 15) if options.idle_only else str(options.seconds * 2 + 180)]
if not options.idle_only:arguments += ["--control-stdin","--no-shell"]
if options.skin:arguments += ["--skin",str(pathlib.Path(options.skin).resolve())]
process=subprocess.Popen(arguments,stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,encoding="utf-8",cwd=root)
replies=queue.Queue()
threading.Thread(target=lambda:[replies.put(json.loads(line)) for line in process.stdout if line.startswith("{")],daemon=True).start()
serial=0
def call(command,args=None):
    global serial
    serial+=1
    process.stdin.write(json.dumps({"id":serial,"command":command,"args":args or {}},ensure_ascii=False)+"\n");process.stdin.flush()
    result=replies.get(timeout=15)
    assert result["id"]==serial,result
    assert not result.get("error"),result
    return result.get("value")
def panel_processes():
    code="Get-CimInstance Win32_Process -Filter \"Name='desktop-pet-panel.exe' OR Name='msedgewebview2.exe'\" | Where-Object { $_.Name -eq 'desktop-pet-panel.exe' -or $_.CommandLine -like '*dev.jarhsonning.mochi.panel*' } | Select-Object ProcessId,ParentProcessId,Name | ConvertTo-Json -Compress"
    result=subprocess.run(["powershell","-NoProfile","-Command",code],capture_output=True,text=True,timeout=12,creationflags=0x08000000)
    text=result.stdout.strip()
    values=json.loads(text) if text else []
    return values if isinstance(values,list) else [values]
class Counters(ctypes.Structure):
    _fields_=[("cb",wintypes.DWORD),("PageFaultCount",wintypes.DWORD)]+[(name,ctypes.c_size_t) for name in ["PeakWorkingSetSize","WorkingSetSize","QuotaPeakPagedPoolUsage","QuotaPagedPoolUsage","QuotaPeakNonPagedPoolUsage","QuotaNonPagedPoolUsage","PagefileUsage","PeakPagefileUsage","PrivateUsage"]]
kernel=ctypes.WinDLL("kernel32",use_last_error=True)
kernel.OpenProcess.restype=wintypes.HANDLE;kernel.OpenProcess.argtypes=[wintypes.DWORD,wintypes.BOOL,wintypes.DWORD]
kernel.GetProcessTimes.argtypes=[wintypes.HANDLE]+[ctypes.POINTER(wintypes.FILETIME)]*4
kernel.CloseHandle.argtypes=[wintypes.HANDLE]
psapi=ctypes.WinDLL("psapi",use_last_error=True)
psapi.GetProcessMemoryInfo.argtypes=[wintypes.HANDLE,ctypes.POINTER(Counters),wintypes.DWORD]
handle=kernel.OpenProcess(0x410,False,process.pid)
assert handle,"Cannot inspect our own pet process"
def sample():
    values=[wintypes.FILETIME() for _ in range(4)]
    assert kernel.GetProcessTimes(handle,*[ctypes.byref(v) for v in values])
    cpu=sum((v.dwHighDateTime<<32)|v.dwLowDateTime for v in values[2:])/10000000
    memory=Counters();memory.cb=ctypes.sizeof(memory)
    assert psapi.GetProcessMemoryInfo(handle,ctypes.byref(memory),memory.cb)
    return {"wall":time.monotonic(),"cpu":cpu,"rss_mib":memory.WorkingSetSize/1048576,"private_mib":memory.PrivateUsage/1048576}
def measure(label):
    start=sample();values=[]
    for _ in range(options.seconds):time.sleep(1);values.append(sample())
    end=values[-1];one_core=(end["cpu"]-start["cpu"])/(end["wall"]-start["wall"])*100
    result={"label":label,"duration":end["wall"]-start["wall"],"one_core_cpu_percent":one_core,"system_cpu_percent":one_core/os.cpu_count(),"rss_mib":sum(v["rss_mib"] for v in values)/len(values),"private_mib":sum(v["private_mib"] for v in values)/len(values),"panel_processes":panel_processes()}
    print(json.dumps(result,ensure_ascii=False),flush=True);return result
report={"passed":False,"pid":process.pid,"skin":options.skin or "builtin","mischief":"Calm","samples":[],"cycles":[]}
try:
    time.sleep(7)
    if not options.idle_only:
        assert call("snapshot")["config"]["clipboard_enabled"] is False
    assert not panel_processes(),"A panel/WebView is already running before opening"
    report["samples"].append(measure("never_opened"))
    for index in range(0 if options.idle_only else 3):
        call("open_panel",{"page":index})
        deadline=time.monotonic()+15
        while time.monotonic()<deadline:
            active=panel_processes()
            if any(p["Name"]=="msedgewebview2.exe" for p in active):break
            time.sleep(.3)
        else:raise AssertionError("Panel did not start")
        time.sleep(4)
        call("close_panel")
        deadline=time.monotonic()+15
        while time.monotonic()<deadline:
            if not panel_processes():break
            time.sleep(.3)
        else:raise AssertionError("Panel/WebView processes remain after close")
        assert process.poll() is None,"Pet exited with panel"
        report["cycles"].append({"cycle":index+1,"released":True,"pet_alive":True})
        print(f"Panel cycle {index+1}: all panel/WebView processes released",flush=True)
    if not options.idle_only:
        time.sleep(3)
        report["samples"].append(measure("after_three_closes"))
        report["rss_delta_mib"]=report["samples"][1]["rss_mib"]-report["samples"][0]["rss_mib"]
        report["private_delta_mib"]=report["samples"][1]["private_mib"]-report["samples"][0]["private_mib"]
        report["cpu_delta_one_core_percent"]=report["samples"][1]["one_core_cpu_percent"]-report["samples"][0]["one_core_cpu_percent"]
        assert not report["samples"][1]["panel_processes"]
        call("quit")
    assert process.wait(timeout=35)==0
    report["passed"]=True
except Exception as error:
    report["error"]=str(error)
    raise
finally:
    if process.poll() is None:process.kill();process.wait()
    kernel.CloseHandle(handle)
    report["stderr"]=process.stderr.read()
    (output/"report.json").write_text(json.dumps(report,ensure_ascii=False,indent=2),encoding="utf-8")
    print(json.dumps(report,ensure_ascii=False,indent=2),flush=True)

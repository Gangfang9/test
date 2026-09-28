"""Real Android 15 overlay/ADB transport check; no cloud membership or touch injection."""
import io
import json
import pathlib
import re
import socket
import subprocess
import sys
import threading
import time
import xml.etree.ElementTree as ET
from PIL import Image, ImageChops

OUT = pathlib.Path('android-smoke-results')
OUT.mkdir(exist_ok=True)
PACKAGE = 'top.jxzs.companion'

def adb(*args):
    return subprocess.check_output(['adb', *args], timeout=30)

def screenshot(name):
    raw = adb('exec-out', 'screencap', '-p')
    (OUT / (name + '.png')).write_bytes(raw)
    return Image.open(io.BytesIO(raw)).convert('RGB')

adb('install', '-r', sys.argv[1])
adb('shell', 'appops', 'set', PACKAGE, 'SYSTEM_ALERT_WINDOW', 'allow')
adb('shell', 'pm', 'grant', PACKAGE, 'android.permission.POST_NOTIFICATIONS')
adb('shell', 'am', 'start', '-W', '-n', PACKAGE + '/.MainActivity')
time.sleep(1)
for _ in range(4):
    adb('shell', 'uiautomator', 'dump', '/sdcard/jxzs-ui.xml')
    tree = ET.fromstring(adb('shell', 'cat', '/sdcard/jxzs-ui.xml'))
    candidates = [n for n in tree.iter('node') if n.get('text') == '启动鼠标服务']
    if candidates:
        bounds = list(map(int, re.findall(r'\d+', candidates[0].get('bounds'))))
        if bounds[3] > bounds[1]:
            adb('shell', 'input', 'tap', str((bounds[0]+bounds[2])//2), str((bounds[1]+bounds[3])//2))
            break
    adb('shell', 'input', 'swipe', '500', '1500', '500', '500', '300')
else:
    raise AssertionError('Start-service button not found')
time.sleep(1)
port = int(adb('forward', 'tcp:0', 'localabstract:jxzs_cursor_v1').strip())
base = screenshot('hidden')
w, h = base.size
connection = socket.create_connection(('127.0.0.1', port), timeout=3)
assert connection.makefile('rb').readline() == b'JXZS/1\n', 'ADB peer handshake failed'
frame = {'v': 1, 'visible': False, 'x': 0.25, 'y': 0.4, 'width': w, 'height': h}
lock = threading.Lock()
running = True

def feed():
    while running:
        with lock: payload = (json.dumps(frame) + '\n').encode()
        try: connection.sendall(payload)
        except OSError: return
        time.sleep(0.03)

thread = threading.Thread(target=feed, daemon=True)
thread.start()
time.sleep(0.3)
base = screenshot('connected-hidden')
with lock: frame['visible'] = True
time.sleep(0.3)
visible = screenshot('visible')
for name, args in [('window.txt', ('dumpsys', 'window', 'windows')), ('logcat.txt', ('logcat', '-d', '-s', 'JXZSPointer:V', 'AndroidRuntime:E'))]:
    (OUT / name).write_bytes(adb('shell', *args))
cx, cy = round(w*0.25), round(h*0.4)
roi = (max(0,cx-3), max(0,cy-3), min(w,cx+100), min(h,cy+120))
assert ImageChops.difference(base.crop(roi), visible.crop(roi)).getbbox(), 'Arrow absent at normalized hotspot'
dump = adb('shell', 'dumpsys', 'window', 'windows').decode(errors='replace')
assert 'JXZS USB Pointer' in dump
with lock: frame['visible'] = False
time.sleep(0.3)
hidden = screenshot('hidden-again')
assert ImageChops.difference(visible.crop(roi), hidden.crop(roi)).getbbox(), 'Toggle did not hide pointer'
with lock: frame['visible'] = True
time.sleep(0.3)
screenshot('before-disconnect')
running = False
thread.join(timeout=1)
# Leave the TCP connection open but stop sending: stale heartbeat must hide.
time.sleep(0.9)
stale = screenshot('heartbeat-expired')
assert ImageChops.difference(visible.crop(roi), stale.crop(roi)).getbbox(), 'Stale pointer remained visible'
connection.close()
adb('forward', '--remove', 'tcp:' + str(port))
crash = adb('logcat', '-d', '-b', 'crash').decode(errors='replace')
assert PACKAGE not in crash, crash
print('Android 15: signed APK install, user service start, ADB peer handshake, normalized arrow, toggle and stale timeout: PASS')

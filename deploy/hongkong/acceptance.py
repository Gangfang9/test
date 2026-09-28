"""One-use authorized live acceptance: creates a marked account and one test card.

Run on the Hong Kong server as root. Credentials never appear in output.
"""
import json
import os
from pathlib import Path
import secrets
import subprocess
import sys
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen

record = Path('/root/jx-acceptance-private.json')
resume = '--resume' in sys.argv
if record.exists() and not resume:
    raise SystemExit('Acceptance already started; inspect existing record before retrying')
account = 'jxqa' + secrets.token_hex(4)
password = secrets.token_urlsafe(12)
device = secrets.token_hex(32)
card = 'JXQA-' + secrets.token_hex(12)
os.umask(0o077)
if resume:
    saved = json.loads(record.read_text())
    account, password, device, card = (saved[k] for k in ('account', 'password', 'device', 'card'))
else:
    record.write_text(json.dumps(dict(account=account, password=password,
                                     device=device, card=card)))
sql = ("INSERT INTO u_cdk_user (gid,type,cdk,val,note,add_role,add_uid,add_time,add_ip,appid) "
       f"VALUES (1,'vip','{card}',86400,'HK migration acceptance only','admin',1,"
       f"{int(time.time())},'127.0.0.1',1000);")
if not resume:
    subprocess.run(['mysql', 'jx_uverif'], input=sql.encode(), check=True,
                   stdout=subprocess.DEVNULL)


def call(path, body, token=None, identity=device):
    headers = {'Content-Type': 'application/json', 'X-Device-ID': identity}
    if token:
        headers['Authorization'] = 'Bearer ' + token
    req = Request('https://www.jxzs.top/v1/' + path,
                  data=json.dumps(body).encode(), headers=headers, method='POST')
    try:
        response = urlopen(req, timeout=12)
    except HTTPError as error:
        response = error
    with response:
        return response.status, json.loads(response.read())


def check(name, condition):
    print(name + ': ' + ('PASS' if condition else 'FAIL'), flush=True)
    if not condition:
        raise SystemExit('Acceptance stopped; private account/card record retained')


credentials = dict(account=account, password=password, device_id=device)
if not resume:
    status, result = call('register', credentials)
    check('register', status == 200 and result.get('ok') is True)
status, result = call('login', credentials)
check('login_without_membership', status == 200 and result.get('ok') is True
      and result.get('data', {}).get('member') is False)
token = result['data']['session']
status, result = call('redeem', {'card': 'INVALID-' + secrets.token_hex(12)}, token)
check('invalid_card_rejected', status >= 400 and result.get('ok') is False)
status, result = call('heartbeat', {}, token)
check('invalid_card_did_not_grant_membership', status == 200
      and result.get('data', {}).get('member') is False)
status, result = call('redeem', {'card': card}, token)
check('valid_card_grants_membership', status == 200
      and result.get('data', {}).get('member') is True)
status, result = call('heartbeat', {}, token)
check('cloud_heartbeat_confirms_membership', status == 200
      and result.get('data', {}).get('member') is True)
status, result = call('redeem', {'card': card}, token)
check('reused_card_rejected', status >= 400 and result.get('ok') is False)
status, result = call('heartbeat', {}, token, secrets.token_hex(32))
check('other_device_rejected', status >= 400 and result.get('ok') is False)
status, result = call('logout', {}, token)
check('logout', status == 200 and result.get('ok') is True)
status, result = call('heartbeat', {}, token)
check('logged_out_session_rejected', status >= 400 and result.get('ok') is False)
query = ("SELECT COUNT(*) FROM u_cdk_user WHERE cdk='" + card
         + "' AND use_uid > 0 AND use_time > 0;")
state = subprocess.check_output(['mysql', '-N', '-B', 'jx_uverif', '-e', query]).strip()
check('uverif_database_records_card_redemption', state == b'1')
print('Live cloud acceptance completed; no credentials printed')

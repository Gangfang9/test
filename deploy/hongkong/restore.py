"""Restore a private migration archive on a fresh Hong Kong host as root.

Run only after MySQL, Redis and Nginx installation has completed.
"""
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tarfile


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


stage = Path('/root/jx-migration-stage')
stage.mkdir(mode=0o700, exist_ok=True)
with tarfile.open('/root/jx-migration-20260928.tar.gz') as archive:
    for member in archive.getmembers():
        if not (stage / member.name).resolve().is_relative_to(stage):
            raise SystemExit('Unsafe archive member')
    archive.extractall(stage, filter='data')
if not shutil.which('mysql'):
    raise SystemExit('Install dependencies first')
for name in ('ubuntu', 'jxmember'):
    if subprocess.run(['id', name], stdout=subprocess.DEVNULL,
                      stderr=subprocess.DEVNULL).returncode:
        run('useradd', '--system', '--create-home', '--shell', '/usr/sbin/nologin', name)
for name in ('jx-uverif', 'jx-membership'):
    destination = Path('/opt') / name
    if destination.exists():
        raise SystemExit('Existing installation requires review')
    shutil.copytree(stage / 'opt' / name, destination)
run('systemctl', 'start', 'mysql', 'redis-server')
backup = stage / 'var/backups/jx-hk-20260928'
for filename in ('user.sql', 'database.sql'):
    with (backup / filename).open('rb') as data:
        run('mysql', stdin=data, stdout=subprocess.DEVNULL)
state = Path('/var/lib/jx-membership')
state.mkdir(mode=0o700, exist_ok=True)
shutil.copy2(backup / 'gateway.sqlite3', state / 'gateway.sqlite3')
with sqlite3.connect(state / 'gateway.sqlite3') as connection:
    # Redis tokens are not portable; users must log in again after migration.
    connection.execute('DELETE FROM sessions')
run('chown', '-R', 'ubuntu:ubuntu', '/opt/jx-uverif')
run('chown', '-R', 'root:jxmember', '/opt/jx-membership')
run('chown', '-R', 'jxmember:jxmember', str(state))
for name in ('jx-membership', 'jx-uverif'):
    unit = Path('etc/systemd/system') / (name + '.service')
    shutil.copy2(stage / unit, Path('/') / unit)
os.chmod('/opt/jx-uverif/config.yaml', 0o600)
os.chmod('/opt/jx-uverif/uverif', 0o700)
os.chmod('/opt/jx-membership/gateway.py', 0o640)
management_sources = os.environ.get('JX_SSH_SOURCE_CIDRS', '').split(',')
if not all(source.strip() for source in management_sources):
    raise SystemExit('Set JX_SSH_SOURCE_CIDRS before enabling the firewall')
for source in management_sources:
    run('ufw', 'allow', 'from', source.strip(), 'to', 'any', 'port', '22', 'proto', 'tcp')
run('ufw', 'allow', '80/tcp')
run('ufw', 'allow', '443/tcp')
run('ufw', '--force', 'enable')
run('systemctl', 'daemon-reload')
run('systemctl', 'enable', '--now', 'jx-uverif', 'jx-membership')
run('systemctl', 'is-active', 'jx-uverif', 'jx-membership')
print('Restore completed; public HTTPS and live card checks still pending')

"""Small, fail-closed HTTP gateway for the private U验证 service.

The U验证 port must remain private. TLS terminates at nginx; this process only
listens on 127.0.0.1. No U验证 credential or token is returned to the client.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import secrets
import sqlite3
import threading
import time
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError, URLError
from urllib.parse import urlencode
from urllib.request import Request, urlopen


APP_ID = os.environ.get("JX_APP_ID", "1000")
VERSION_KEY = os.environ.get("JX_VERSION_KEY", "windows")
VERSION = os.environ.get("JX_VERSION", "1.0.0")
U_BASE = os.environ.get("JX_U_BASE", "http://127.0.0.1:19080").rstrip("/")
DB_PATH = os.environ.get("JX_GATEWAY_DB", "/var/lib/jx-membership/gateway.sqlite3")
ACCOUNT_RE = re.compile(r"^[A-Za-z0-9_]{6,18}$")
DEVICE_RE = re.compile(r"^[A-Za-z0-9_.:-]{16,64}$")
TOKEN_RE = re.compile(r"^[A-Za-z0-9_-]{32,128}$")
CARD_RE = re.compile(r"^[A-Za-z0-9-]{6,128}$")
LOCK = threading.RLock()


@contextmanager
def db():
    connection = sqlite3.connect(DB_PATH, timeout=5)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA busy_timeout=5000")
    try:
        with connection:
            yield connection
    finally:
        connection.close()


def initialize_db():
    os.makedirs(os.path.dirname(DB_PATH), mode=0o700, exist_ok=True)
    with db() as connection:
        connection.execute("PRAGMA journal_mode=WAL")
        connection.executescript("""
            CREATE TABLE IF NOT EXISTS registrations (
                device_hash TEXT NOT NULL,
                ip_hash TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS registration_device_time
                ON registrations(device_hash, created_at);
            CREATE TABLE IF NOT EXISTS attempts (
                scope TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS attempt_scope_time
                ON attempts(scope, created_at);
            CREATE TABLE IF NOT EXISTS sessions (
                token_hash TEXT PRIMARY KEY,
                u_token TEXT NOT NULL,
                device_hash TEXT NOT NULL,
                account TEXT NOT NULL,
                member INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                last_checked INTEGER NOT NULL
            );
        """)
        # Upgrade databases created before per-IP successful-registration limits.
        columns = {row[1] for row in connection.execute("PRAGMA table_info(registrations)")}
        if "ip_hash" not in columns:
            connection.execute("ALTER TABLE registrations ADD COLUMN ip_hash TEXT")
        connection.execute(
            "CREATE INDEX IF NOT EXISTS registration_ip_time ON registrations(ip_hash, created_at)"
        )


def digest(value):
    return hashlib.sha256(value.encode("utf-8")).hexdigest()


class GatewayError(Exception):
    def __init__(self, status, message):
        super().__init__(message)
        self.status = status
        self.message = message


def u_request(method, fields=None):
    url = f"{U_BASE}/api/user/{APP_ID}/{VERSION_KEY}/{VERSION}/{method}"
    data = None if fields is None else urlencode(fields).encode("utf-8")
    request = Request(url, data=data, method="GET" if data is None else "POST")
    if data is not None:
        request.add_header("Content-Type", "application/x-www-form-urlencoded")
    try:
        with urlopen(request, timeout=5) as response:
            if response.status != 200:
                raise GatewayError(503, "验证服务暂不可用")
            payload = response.read(65537)
            if len(payload) > 65536:
                raise GatewayError(503, "验证服务响应异常")
            result = json.loads(payload)
            if not isinstance(result, dict) or not isinstance(result.get("code"), int):
                raise ValueError("invalid U response")
            return result
    except (HTTPError, URLError, TimeoutError, ValueError) as error:
        raise GatewayError(503, "验证服务暂不可用") from error


def require_success(result, default="验证失败"):
    if result.get("code") != 0:
        # Keep upstream messages short, but never expose raw server responses.
        message = result.get("msg")
        if not isinstance(message, str) or len(message) > 100:
            message = default
        raise GatewayError(403, message)


def limited(scope, limit, period):
    now = int(time.time())
    with LOCK, db() as connection:
        connection.execute("BEGIN IMMEDIATE")
        connection.execute("DELETE FROM attempts WHERE created_at < ?", (now - 86400,))
        count = connection.execute(
            "SELECT COUNT(*) FROM attempts WHERE scope=? AND created_at>=?",
            (scope, now - period),
        ).fetchone()[0]
        if count >= limit:
            raise GatewayError(429, "操作过于频繁，请稍后重试")
        connection.execute(
            "INSERT INTO attempts(scope, created_at) VALUES (?,?)", (scope, now)
        )


def validate(body, key, pattern=None, min_len=None, max_len=None):
    value = body.get(key)
    if not isinstance(value, str):
        raise GatewayError(400, f"{key} 格式错误")
    if key == "device_id" and len(value) > 64:
        raise GatewayError(400, "device_id 长度不能大于 64")
    if pattern and not pattern.fullmatch(value):
        raise GatewayError(400, f"{key} 格式错误")
    if min_len is not None and len(value) < min_len:
        raise GatewayError(400, f"{key} 格式错误")
    if max_len is not None and len(value) > max_len:
        raise GatewayError(400, f"{key} 格式错误")
    return value


def register(body, ip):
    account = validate(body, "account", ACCOUNT_RE)
    password = validate(body, "password", min_len=6, max_len=18)
    device = validate(body, "device_id", DEVICE_RE)
    device_hash = digest(device)
    ip_hash = digest(ip)
    limited("register-ip:" + digest(ip), 20, 3600)
    now = int(time.time())
    # Serialize the complete check-and-register sequence. A failed upstream
    # registration does not consume the 24-hour successful-registration slot.
    with LOCK, db() as connection:
        connection.execute("BEGIN IMMEDIATE")
        existing = connection.execute(
            "SELECT 1 FROM registrations WHERE device_hash=? AND created_at>=? LIMIT 1",
            (device_hash, now - 86400),
        ).fetchone()
        if existing:
            raise GatewayError(429, "这台电脑 24 小时内已注册过账号")
        ip_count = connection.execute(
            "SELECT COUNT(*) FROM registrations WHERE ip_hash=? AND created_at>=?",
            (ip_hash, now - 86400),
        ).fetchone()[0]
        if ip_count >= 10:
            raise GatewayError(429, "该网络注册过于频繁，请稍后重试")
        result = u_request("reg", {"account": account, "password": password, "udid": device})
        require_success(result, "注册失败")
        connection.execute(
            "INSERT INTO registrations(device_hash, ip_hash, created_at) VALUES (?,?,?)",
            (device_hash, ip_hash, now),
        )
    return {"registered": True}


def session_for(token, device):
    if not TOKEN_RE.fullmatch(token or "") or not DEVICE_RE.fullmatch(device or ""):
        raise GatewayError(401, "请重新登录")
    with db() as connection:
        row = connection.execute(
            "SELECT * FROM sessions WHERE token_hash=?", (digest(token),)
        ).fetchone()
    if row is None or row["expires_at"] <= int(time.time()) or row["device_hash"] != digest(device):
        raise GatewayError(401, "登录已失效，请重新登录")
    return row


def require_member(row):
    if not row["member"]:
        raise GatewayError(403, "会员未开通或已到期，请充值卡密")


def membership_data(u_token, info=None):
    """Return U验证's actual expiry, never the gateway session's 24-hour TTL."""
    if info is None:
        result = u_request("info", {"token": u_token})
        require_success(result, "会员信息获取失败")
        info = result.get("data")
    expiry = info.get("vipExpTime") if isinstance(info, dict) else None
    if not isinstance(expiry, int) or isinstance(expiry, bool) or expiry < 0:
        raise GatewayError(503, "会员到期时间响应异常")
    member = expiry > time.time()
    if member:
        member = u_request("vip", {"token": u_token}).get("code") == 0
    now = time.time()
    return {"member": member and expiry > now, "membership_expires_at": expiry,
            "server_time": now}


def login(body, ip):
    account = validate(body, "account", ACCOUNT_RE)
    password = validate(body, "password", min_len=6, max_len=18)
    device = validate(body, "device_id", DEVICE_RE)
    limited("login-ip:" + digest(ip), 30, 3600)
    limited("login-account:" + digest(account.lower()), 10, 600)
    result = u_request("logon", {"account": account, "password": password, "udid": device})
    require_success(result, "账号或密码错误")
    data = result.get("data")
    if not isinstance(data, dict) or not isinstance(data.get("token"), str):
        raise GatewayError(503, "登录响应异常")
    # U验证 returns state=n when this device is not bound. Never bypass that
    # restriction or perform self-service unbinding.
    if data.get("state") != "y":
        raise GatewayError(403, "设备未绑定或已在其他电脑绑定，请联系管理员")
    u_token = data["token"]
    membership = membership_data(u_token, data.get("info"))
    token = secrets.token_urlsafe(48)
    now = int(time.time())
    with LOCK, db() as connection:
        # U验证 alone owns binding/administrator unbinding. Only a successful
        # bound login reaches here. Replace old sessions so an administrator's
        # legitimate device change is not blocked by stale gateway sessions.
        connection.execute("DELETE FROM sessions WHERE account=? COLLATE NOCASE", (account,))
        connection.execute(
            "INSERT INTO sessions VALUES (?,?,?,?,?,?,?)",
            (digest(token), u_token, digest(device), account,
             int(membership["member"]), now + 86400, now),
        )
    return {"session": token, "expires_in": 86400, **membership}


def heartbeat(token, device):
    row = session_for(token, device)
    result = u_request("heartbeat", {"token": row["u_token"]})
    # The installed 3.3.21 server returns code=0 for a valid heartbeat.
    # Older documentation shows 1000, so reject all other codes here.
    if result.get("code") != 0:
        with db() as connection:
            connection.execute("DELETE FROM sessions WHERE token_hash=?", (digest(token),))
        raise GatewayError(401, "登录验证失败，请重新登录")
    membership = membership_data(row["u_token"])
    now = int(time.time())
    with db() as connection:
        connection.execute(
            "UPDATE sessions SET member=?, expires_at=?, last_checked=? WHERE token_hash=?",
            (int(membership["member"]), now + 86400, now, digest(token)),
        )
    return membership


def redeem(token, device, body):
    row = session_for(token, device)
    card = validate(body, "card", CARD_RE)
    limited("redeem:" + digest(row["account"]), 10, 3600)
    require_success(u_request("kamiTopup", {"token": row["u_token"], "kami": card}), "卡密充值失败")
    membership = membership_data(row["u_token"])
    with db() as connection:
        connection.execute("UPDATE sessions SET member=? WHERE token_hash=?",
                           (int(membership["member"]), digest(token)))
    return membership


def logout(token, device):
    row = session_for(token, device)
    try:
        u_request("logout", {"token": row["u_token"]})
    finally:
        with db() as connection:
            connection.execute("DELETE FROM sessions WHERE token_hash=?", (digest(token),))
    return {"logged_out": True}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def respond(self, status, data):
        body = json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path == "/healthz":
            self.respond(200, {"ok": True})
        else:
            self.respond(404, {"error": "not found"})

    def do_POST(self):
        routes = {
            "/v1/register": lambda b, t, d, ip: register(b, ip),
            "/v1/login": lambda b, t, d, ip: login(b, ip),
            "/v1/heartbeat": lambda b, t, d, ip: heartbeat(t, d),
            "/v1/redeem": lambda b, t, d, ip: redeem(t, d, b),
            "/v1/logout": lambda b, t, d, ip: logout(t, d),
        }
        route = routes.get(self.path)
        if route is None:
            self.respond(404, {"error": "not found"})
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if length < 2 or length > 8192:
                raise GatewayError(400, "请求大小错误")
            if self.headers.get("Content-Type", "").split(";")[0] != "application/json":
                raise GatewayError(415, "仅支持 JSON")
            body = json.loads(self.rfile.read(length))
            if not isinstance(body, dict):
                raise GatewayError(400, "请求格式错误")
            token = self.headers.get("Authorization", "").removeprefix("Bearer ")
            device = self.headers.get("X-Device-ID", "")
            # Only nginx may reach this listener. It overwrites X-Real-IP.
            ip = self.headers.get("X-Real-IP", self.client_address[0])
            if len(ip) > 64:
                raise GatewayError(400, "请求格式错误")
            result = route(body, token, device, ip)
            self.respond(200, {"ok": True, "data": result})
        except GatewayError as error:
            self.respond(error.status, {"ok": False, "error": error.message})
        except (ValueError, json.JSONDecodeError):
            self.respond(400, {"ok": False, "error": "请求格式错误"})
        except Exception:
            self.respond(503, {"ok": False, "error": "服务暂不可用"})


if __name__ == "__main__":
    os.umask(0o077)
    initialize_db()
    server = ThreadingHTTPServer(("127.0.0.1", int(os.environ.get("JX_GATEWAY_PORT", "19081"))), Handler)
    server.serve_forever()

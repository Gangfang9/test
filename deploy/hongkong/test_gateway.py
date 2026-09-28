"""Isolated membership regression tests. No live users or credentials."""
import importlib.util
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("jx_gateway", Path(__file__).with_name("gateway.py"))
gateway = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gateway)


class MembershipTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.database = patch.object(gateway, "DB_PATH", str(Path(self.directory.name) / "gateway.sqlite3"))
        self.database.start()
        gateway.initialize_db()
        self.expiry = int(time.time()) + 3600
        self.device = "a" * 64
        self.bound_device = self.device
        self.token_counter = 0

    def tearDown(self):
        self.database.stop()
        self.directory.cleanup()

    def upstream(self, method, fields):
        if method == "logon":
            if fields["udid"] != self.bound_device:
                return {"code": -1, "msg": "机器码绑定上限，请解绑后再操作"}
            self.token_counter += 1
            return {"code": 0, "data": {"state": "y", "token": f"private-{self.token_counter}",
                                      "info": {"vipExpTime": self.expiry}}}
        if method == "info":
            return {"code": 0, "data": {"vipExpTime": self.expiry}}
        if method == "kamiTopup":
            if fields["kami"] == "INVALID-CARD":
                return {"code": -1, "msg": "卡密不存在"}
            self.expiry += 86400
        return {"code": 0}

    def login(self, device=None, account="testmember"):
        return gateway.login({"account": account, "password": "Test1234",
                              "device_id": device or self.device}, "127.0.0.1")

    def test_login_heartbeat_recharge_share_uverif_expiry(self):
        with patch.object(gateway, "u_request", self.upstream):
            login = self.login()
            token = login["session"]
            self.assertTrue(login["member"])
            self.assertEqual(login["membership_expires_at"], self.expiry)
            heartbeat = gateway.heartbeat(token, self.device)
            self.assertEqual(heartbeat["membership_expires_at"], self.expiry)
            previous_expiry = self.expiry
            with self.assertRaises(gateway.GatewayError):
                gateway.redeem(token, self.device, {"card": "INVALID-CARD"})
            self.assertEqual(self.expiry, previous_expiry)
            recharge = gateway.redeem(token, self.device, {"card": "VALID-CARD"})
            self.assertEqual(recharge["membership_expires_at"], previous_expiry + 86400)
            self.assertEqual(gateway.heartbeat(token, self.device)["membership_expires_at"], recharge["membership_expires_at"])
            self.expiry = int(time.time()) - 1
            self.assertFalse(gateway.heartbeat(token, self.device)["member"])

    def test_other_device_blocked_and_admin_unbinding_invalidates_old_session(self):
        with patch.object(gateway, "u_request", self.upstream):
            first = self.login()["session"]
            other = "b" * 64
            with self.assertRaises(gateway.GatewayError):
                self.login(other)
            self.assertTrue(gateway.heartbeat(first, self.device)["member"])
            with self.assertRaises(gateway.GatewayError):
                gateway.heartbeat(first, other)
            # Only the upstream administrator can change binding.
            self.bound_device = other
            replacement = self.login(other)["session"]
            with self.assertRaises(gateway.GatewayError):
                gateway.heartbeat(first, self.device)
            self.assertTrue(gateway.heartbeat(replacement, other)["member"])

    def test_relogin_replaces_old_token_and_logout_revokes_current_token(self):
        with patch.object(gateway, "u_request", self.upstream):
            first = self.login()["session"]
            second = self.login(account="TESTMEMBER")["session"]
            with self.assertRaises(gateway.GatewayError):
                gateway.heartbeat(first, self.device)
            gateway.logout(second, self.device)
            with self.assertRaises(gateway.GatewayError):
                gateway.heartbeat(second, self.device)

    def test_malformed_expiry_cannot_grant_membership(self):
        with patch.object(gateway, "u_request", self.upstream):
            for expiry in [None, True, "9999999999", -1]:
                with self.assertRaises(gateway.GatewayError):
                    gateway.membership_data("private", {"vipExpTime": expiry})


if __name__ == "__main__":
    unittest.main()

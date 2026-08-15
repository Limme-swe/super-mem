from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "check_update.py"
SPEC = importlib.util.spec_from_file_location("check_update", SCRIPT)
assert SPEC and SPEC.loader
check_update = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = check_update
SPEC.loader.exec_module(check_update)


class ReleaseHandler(BaseHTTPRequestHandler):
    payload = {"tag_name": "v0.2.0", "html_url": "https://example.invalid/release"}

    def do_GET(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler contract
        encoded = json.dumps(self.payload).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, *_args: object) -> None:
        return


class CheckUpdateTests(unittest.TestCase):
    def setUp(self) -> None:
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), ReleaseHandler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        host, port = self.server.server_address
        self.url = f"http://{host}:{port}/latest"

    def tearDown(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)

    def test_reports_update_without_treating_it_as_an_error(self) -> None:
        self.assertEqual(
            check_update.main(["--current", "0.1.0", "--api-url", self.url, "--json"]),
            0,
        )

    def test_require_current_uses_a_distinct_exit_code(self) -> None:
        self.assertEqual(
            check_update.main(
                ["--current", "0.1.0", "--api-url", self.url, "--require-current"]
            ),
            10,
        )

    def test_equal_version_is_current(self) -> None:
        self.assertEqual(check_update.version_key("0.2.0"), check_update.version_key("0.2.0+build.1"))
        latest, _ = check_update.latest_release(self.url, 2)
        self.assertFalse(check_update.version_key(latest) > check_update.version_key("0.2.0"))

    def test_reads_version_from_a_binary(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "supermem"
            binary.write_text("#!/bin/sh\necho 'supermem 0.1.7'\n", encoding="utf-8")
            binary.chmod(0o755)
            self.assertEqual(check_update.installed_version(str(binary)), "0.1.7")

    def test_invalid_versions_are_rejected(self) -> None:
        with self.assertRaises(ValueError):
            check_update.normalized("nightly")


if __name__ == "__main__":
    unittest.main()

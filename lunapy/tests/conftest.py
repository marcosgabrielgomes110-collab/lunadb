from __future__ import annotations

import os
import socket
import subprocess
import time
import urllib.request
import uuid
from pathlib import Path

import pytest

from lunapy import LunaClient, LunaNotFoundError

REPO_ROOT = Path(__file__).resolve().parents[2]
ROOT_PASSWORD = "pytest-secret"


def _free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def _wait_health(base_url: str, process: subprocess.Popen, timeout: float = 15.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if process.poll() is not None:
            raise RuntimeError("server died during boot")
        try:
            with urllib.request.urlopen(base_url + "/health", timeout=0.5) as response:
                if response.status == 200:
                    return
        except Exception:
            time.sleep(0.1)
    process.kill()
    raise RuntimeError("server did not respond to /health in time")


@pytest.fixture(scope="session")
def server(tmp_path_factory: pytest.TempPathFactory) -> dict:
    binary = Path(os.environ.get("LUNADB_BIN", REPO_ROOT / "target" / "debug" / "lunadb"))
    if not binary.exists():
        pytest.skip(f"binary not found at {binary} (run cargo build)")

    home = tmp_path_factory.mktemp("lunadb")
    storage = home / "storage"
    storage.mkdir()
    port = _free_port()
    config = home / "config.toml"
    config.write_text(
        f"""
[server]
host = "127.0.0.1"
port = {port}

[storage]
path = "{storage}"

[root]
username = "root"
password = "{ROOT_PASSWORD}"
"""
    )
    process = subprocess.Popen(
        [str(binary), "--config", str(config), "serve"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    base_url = f"http://127.0.0.1:{port}"
    _wait_health(base_url, process)
    yield {
        "base_url": base_url,
        "home": home,
        "storage": storage,
        "config": config,
    }
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()


@pytest.fixture()
def admin(server: dict) -> LunaClient:
    client = LunaClient(base_url=server["base_url"])
    client.login("root", ROOT_PASSWORD)
    return client


@pytest.fixture()
def db(admin: LunaClient) -> str:
    name = "t" + uuid.uuid4().hex[:10]
    admin.create_db(name)
    yield name
    try:
        admin.delete_db(name)
    except LunaNotFoundError:
        pass

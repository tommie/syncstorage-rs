#!/usr/bin/env python3

import errno
import os.path
import psutil
import signal
import subprocess
import sys
from urllib.error import URLError
from urllib.parse import urljoin
from urllib.request import urlopen
from test_storage import TestStorage
from test_support import parse_test_args, run_live_functional_tests
import time
from tokenserver.run import (run_end_to_end_tests, run_local_tests)

DEBUG_BUILD = "target/debug/syncserver"
RELEASE_BUILD = "/app/bin/syncserver"


def terminate_process(process):
    proc = psutil.Process(pid=process.pid)
    child_proc = proc.children(recursive=True)
    for p in [proc] + child_proc:
        os.kill(p.pid, signal.SIGTERM)
    process.wait()


def ping_http(url: str):
    try:
        resp = urlopen(url, timeout=1)
        return resp.status // 100 == 2
    except URLError as ex:
        if (not isinstance(ex.reason, OSError) or
                ex.reason.errno != errno.EADDRNOTAVAIL):
            print(ex, file=sys.stderr)
        return False


if __name__ == "__main__":
    # When run as a script, this file will execute the
    # functional tests against a live webserver.
    target_binary = None
    if os.path.exists(DEBUG_BUILD):
        target_binary = DEBUG_BUILD
    elif os.path.exists(RELEASE_BUILD):
        target_binary = RELEASE_BUILD
    else:
        raise RuntimeError(
            "Neither target/debug/syncserver \
                nor /app/bin/syncserver were found."
        )

    def start_server():
        opts, args = parse_test_args(sys.argv)

        the_server_subprocess = subprocess.Popen(
            [target_binary], env=os.environ
        )

        heartbeat_url = urljoin(args[1], "/__heartbeat__")
        while the_server_subprocess.poll() is None:
            time.sleep(1)
            if ping_http(heartbeat_url):
                break

        if the_server_subprocess.returncode is not None:
            raise subprocess.CalledProcessError(
                the_server_subprocess.returncode,
                the_server_subprocess.args,
                the_server_subprocess.stdout,
                the_server_subprocess.stderr)

        return the_server_subprocess

    os.environ.setdefault("SYNC_MASTER_SECRET", "secret0")
    os.environ.setdefault("SYNC_CORS_MAX_AGE", "555")
    os.environ.setdefault("SYNC_CORS_ALLOWED_ORIGIN", "*")
    mock_fxa_server_url = os.environ["MOCK_FXA_SERVER_URL"]
    url = "%s/v2" % mock_fxa_server_url
    os.environ["SYNC_TOKENSERVER__FXA_OAUTH_SERVER_URL"] = mock_fxa_server_url
    the_server_subprocess = start_server()
    try:
        res = 0
        res |= run_live_functional_tests(TestStorage, sys.argv)
        os.environ["TOKENSERVER_AUTH_METHOD"] = "oauth"
        res |= run_local_tests()
    finally:
        terminate_process(the_server_subprocess)

    os.environ["SYNC_TOKENSERVER__FXA_OAUTH_SERVER_URL"] = \
        "https://oauth.stage.mozaws.net"
    the_server_subprocess = start_server()
    try:
        res |= run_end_to_end_tests()
    finally:
        terminate_process(the_server_subprocess)

    # Run the Tokenserver end-to-end tests without the JWK cached
    del os.environ["SYNC_TOKENSERVER__FXA_OAUTH_PRIMARY_JWK__KTY"]
    del os.environ["SYNC_TOKENSERVER__FXA_OAUTH_PRIMARY_JWK__ALG"]
    del os.environ["SYNC_TOKENSERVER__FXA_OAUTH_PRIMARY_JWK__KID"]
    del os.environ["SYNC_TOKENSERVER__FXA_OAUTH_PRIMARY_JWK__FXA_CREATED_AT"]
    del os.environ["SYNC_TOKENSERVER__FXA_OAUTH_PRIMARY_JWK__USE"]
    del os.environ["SYNC_TOKENSERVER__FXA_OAUTH_PRIMARY_JWK__N"]
    del os.environ["SYNC_TOKENSERVER__FXA_OAUTH_PRIMARY_JWK__E"]

    the_server_subprocess = start_server()
    try:
        verbosity = int(os.environ.get("VERBOSITY", "1"))
        res |= run_end_to_end_tests(verbosity=verbosity)
    finally:
        terminate_process(the_server_subprocess)

    sys.exit(res)

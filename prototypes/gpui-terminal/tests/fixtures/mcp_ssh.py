"""Loopback-only SSH/SFTP fixture. Its shell evaluates only the cases in this file."""

import errno
import json
import os
from pathlib import Path
import re
import shlex
import socket
import sys
import tempfile
import threading
import time

import paramiko


class Files(paramiko.SFTPServerInterface):
    def __init__(self, server, root):
        super().__init__(server)
        self.root = root

    def local(self, path):
        result = (self.root / path.lstrip("/")).resolve()
        if not result.is_relative_to(self.root):
            raise OSError(errno.EACCES, "Outside fixture")
        return result

    def stat(self, path):
        try:
            return paramiko.SFTPAttributes.from_stat(self.local(path).stat())
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    lstat = stat

    def open(self, path, flags, _attr):
        try:
            mode = "wb" if flags & os.O_WRONLY else "rb"
            local = self.local(path)
            fd = os.open(local, flags | getattr(os, "O_BINARY", 0), 0o600)
            stream = os.fdopen(fd, mode)
            handle = paramiko.SFTPHandle(flags)
            handle.readfile = stream
            handle.writefile = stream
            handle.stat = lambda: paramiko.SFTPAttributes.from_stat(os.fstat(stream.fileno()))
            return handle
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)

    def remove(self, path):
        try:
            self.local(path).unlink()
            return paramiko.SFTP_OK
        except OSError as error:
            return paramiko.SFTPServer.convert_errno(error.errno)


MARKER = re.compile(rb"; command printf '\\033\]633;OrbitShell;([0-9a-f]+);%s\\007'")


def run_shell(channel, ordinal):
    cwd = "/original" if ordinal == 1 else "/dedicated"
    data = b""
    try:
        channel.sendall(f"fixture-terminal-{ordinal}\r\n".encode())
        while chunk := channel.recv(4096):
            data += chunk
            if data == b"\x03":
                data = b""
                channel.sendall(b"fixture-interrupted\r\n")
            while b"\r" in data:
                line, data = data.split(b"\r", 1)
                marker = MARKER.search(line)
                command = line.decode() if marker is None else shlex.split(line[5:marker.start()].decode().replace("\\\n", ""))[0]
                code = 0
                if command == "pwd":
                    channel.sendall(f"{cwd}\r\n".encode())
                elif command == "cd /fixture":
                    cwd = "/fixture"
                elif command == "printf fixture-pty":
                    channel.sendall(b"fixture-pty\r\n")
                elif command == "false":
                    code = 1
                elif command == "sleep 2":
                    time.sleep(1.6)
                    channel.sendall(b"fixture-finished\r\n")
                else:
                    code = 127
                    channel.sendall(b"fixture-command-unrecognized\r\n")
                if marker:
                    notice = b"\x1b]633;OrbitShell;" + marker[1] + b";" + str(code).encode() + b"\x07"
                    channel.sendall(notice[:9])
                    channel.sendall(notice[9:])
    except (EOFError, OSError):
        pass


class Server(paramiko.ServerInterface):
    def __init__(self):
        self.shells = 0

    def get_allowed_auths(self, _username):
        return "password"

    def check_auth_password(self, username, password):
        if (username, password) == ("fixture", "fixtures-only"):
            return paramiko.AUTH_SUCCESSFUL
        return paramiko.AUTH_FAILED

    def check_channel_request(self, kind, _channel_id):
        return paramiko.OPEN_SUCCEEDED if kind == "session" else paramiko.OPEN_FAILED_ADMINISTRATIVELY_PROHIBITED

    def check_channel_pty_request(self, *_args):
        return True

    def check_channel_window_change_request(self, *_args):
        return True

    def check_channel_shell_request(self, channel):
        self.shells += 1
        threading.Thread(target=run_shell, args=(channel, self.shells), daemon=True).start()
        return True


def serve(listener, root):
    host_key = paramiko.RSAKey.generate(2048)
    while True:
        try:
            connection, _ = listener.accept()
        except OSError:
            return
        transport = paramiko.Transport(connection)
        transport.add_server_key(host_key)
        transport.set_subsystem_handler("sftp", paramiko.SFTPServer, Files, root)
        transport.start_server(server=Server())
        while transport.is_active():
            # Retain accepted channels; otherwise Paramiko may collect them before the fixture thread starts.
            channel = transport.accept(0.2)
            if channel is not None:
                channels.append(channel)


if __name__ == "__main__":
    channels = []
    with tempfile.TemporaryDirectory(prefix="orbitshell-mcp-") as directory, socket.socket() as listener:
        root = Path(directory).resolve()
        remote = root / "remote"
        remote.mkdir()
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        threading.Thread(target=serve, args=(listener, remote), daemon=True).start()
        print(json.dumps({"root": str(root), "port": listener.getsockname()[1]}), flush=True)
        sys.stdin.buffer.read()

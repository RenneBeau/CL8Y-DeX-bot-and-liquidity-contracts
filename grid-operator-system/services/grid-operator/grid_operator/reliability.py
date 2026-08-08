import fcntl
import os


class StateLockError(RuntimeError):
    pass


class StateIdentityError(RuntimeError):
    pass


class ProcessLock:
    """Nonblocking process lock held by an open file descriptor."""

    def __init__(self, state_path: str):
        self.path = os.path.abspath(state_path) + ".lock"
        directory = os.path.dirname(self.path)
        os.makedirs(directory, mode=0o700, exist_ok=True)
        self.fd: int | None = os.open(self.path, os.O_RDWR | os.O_CREAT, 0o600)
        try:
            fcntl.flock(self.fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as exc:
            os.close(self.fd)
            self.fd = None
            raise StateLockError(f"state is already locked by another process: {state_path}") from exc

    def close(self):
        if self.fd is not None:
            os.close(self.fd)
            self.fd = None

    def __enter__(self):
        return self

    def __exit__(self, *_args):
        self.close()

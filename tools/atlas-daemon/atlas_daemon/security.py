from __future__ import annotations

from cryptography.fernet import Fernet

_fernet: Fernet | None = None


def init_encryption(key: str) -> None:
    global _fernet
    if not key:
        _fernet = Fernet(Fernet.generate_key())
        return
    if len(key) == 44 and key.endswith("="):
        _fernet = Fernet(key.encode())
    else:
        import base64
        import hashlib

        derived = hashlib.sha256(key.encode()).digest()
        _fernet = Fernet(base64.urlsafe_b64encode(derived))


def encrypt_api_key(plaintext: str) -> bytes:
    if _fernet is None:
        raise RuntimeError("Encryption not initialized. Call init_encryption() first.")
    return _fernet.encrypt(plaintext.encode())


def decrypt_api_key(ciphertext: bytes) -> str:
    if _fernet is None:
        raise RuntimeError("Encryption not initialized. Call init_encryption() first.")
    return _fernet.decrypt(ciphertext).decode()

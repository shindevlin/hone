"""Thin async HTTP client for a hone-node's local HTTP API.

This talks ONLY to the node named by HONE_NODE_URL (default:
http://127.0.0.1:4242 — hone-node's documented default HONE_API_PORT).
It is a dumb pass-through: it does not interpret chain semantics beyond
what's needed to shape request bodies the API already documents in its
own handler doc-comments (see rust/hone-node/src/api.rs in the hone repo).
"""

from __future__ import annotations

import os
from typing import Any

import httpx

DEFAULT_NODE_URL = "http://127.0.0.1:4242"
HUNITS_PER_HONE = 10_000_000_000


def hone_to_hunits(amount_hone: float) -> int:
    """Convert a HONE amount (float, as a human would type it) to integer hunits.

    Hone's ledger amounts (`Hunits`) are u64 integers — never floats — so this
    always rounds to the nearest hunit rather than passing a float downstream.
    """
    return round(amount_hone * HUNITS_PER_HONE)


def hunits_to_hone(amount_hunits: int) -> float:
    return amount_hunits / HUNITS_PER_HONE


class HoneApiError(RuntimeError):
    """Raised when the node's API returns a non-2xx response or a transport error."""

    def __init__(self, message: str, status_code: int | None = None, body: Any = None):
        super().__init__(message)
        self.status_code = status_code
        self.body = body


class HoneClient:
    """Async client bound to one hone-node instance's local HTTP API."""

    def __init__(self, base_url: str | None = None, timeout: float = 15.0):
        self.base_url = (base_url or os.environ.get("HONE_NODE_URL") or DEFAULT_NODE_URL).rstrip("/")
        self._timeout = timeout

    async def get(self, path: str, params: dict[str, Any] | None = None) -> Any:
        async with httpx.AsyncClient(timeout=self._timeout) as client:
            try:
                resp = await client.get(f"{self.base_url}{path}", params=params)
            except httpx.RequestError as e:
                raise HoneApiError(
                    f"could not reach hone-node at {self.base_url}{path}: {e}"
                ) from e
        return self._unwrap(resp)

    async def post(self, path: str, json_body: dict[str, Any]) -> Any:
        async with httpx.AsyncClient(timeout=self._timeout) as client:
            try:
                resp = await client.post(f"{self.base_url}{path}", json=json_body)
            except httpx.RequestError as e:
                raise HoneApiError(
                    f"could not reach hone-node at {self.base_url}{path}: {e}"
                ) from e
        return self._unwrap(resp)

    @staticmethod
    def _unwrap(resp: httpx.Response) -> Any:
        try:
            body = resp.json()
        except ValueError:
            body = resp.text
        if resp.status_code >= 400:
            msg = body.get("error") if isinstance(body, dict) else None
            raise HoneApiError(
                msg or f"hone-node returned HTTP {resp.status_code}",
                status_code=resp.status_code,
                body=body,
            )
        return body

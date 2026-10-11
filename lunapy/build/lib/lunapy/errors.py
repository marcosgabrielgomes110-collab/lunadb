from __future__ import annotations

from typing import Any, Optional


class LunaError(Exception):
    pass


class LunaConnectionError(LunaError):
    pass


class LunaHTTPError(LunaError):
    def __init__(
        self,
        status: int,
        message: str,
        payload: Optional[dict[str, Any]] = None,
    ) -> None:
        self.status = status
        self.message = message
        self.payload = payload or {}
        super().__init__(f"HTTP {status}: {message}")


class LunaBadRequestError(LunaHTTPError):
    pass


class LunaValidationError(LunaBadRequestError):
    def __init__(
        self,
        status: int,
        message: str,
        payload: Optional[dict[str, Any]] = None,
    ) -> None:
        super().__init__(status, message, payload)
        self.fields = self.payload.get("fields", [])


class LunaAuthError(LunaHTTPError):
    pass


class LunaForbiddenError(LunaHTTPError):
    pass


class LunaNotFoundError(LunaHTTPError):
    pass


class LunaConflictError(LunaHTTPError):
    pass


class LunaUniqueViolationError(LunaConflictError):
    def __init__(
        self,
        status: int,
        message: str,
        payload: Optional[dict[str, Any]] = None,
    ) -> None:
        super().__init__(status, message, payload)
        self.fields = self.payload.get("fields", [])


class LunaSchemaRejectedError(LunaConflictError):
    def __init__(
        self,
        status: int,
        message: str,
        payload: Optional[dict[str, Any]] = None,
    ) -> None:
        super().__init__(status, message, payload)
        self.records = self.payload.get("records", [])


_STATUS_MAP: dict[int, type[LunaHTTPError]] = {
    400: LunaBadRequestError,
    401: LunaAuthError,
    403: LunaForbiddenError,
    404: LunaNotFoundError,
    409: LunaConflictError,
}


def error_from_status(
    status: int,
    message: str,
    payload: Optional[dict[str, Any]] = None,
) -> LunaHTTPError:
    payload = payload or {}
    if status == 400 and payload.get("fields"):
        return LunaValidationError(status, message, payload)
    if status == 409 and payload.get("records"):
        return LunaSchemaRejectedError(status, message, payload)
    if status == 409 and payload.get("fields"):
        return LunaUniqueViolationError(status, message, payload)
    return _STATUS_MAP.get(status, LunaHTTPError)(status, message, payload)

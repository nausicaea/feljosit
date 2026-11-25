"""Errors that the services raise."""


class ServiceError(ValueError):
    """The operation is not permitted or the input is not valid.

    The message is safe to show to the user.
    """


class NotFoundError(LookupError):
    """The object does not exist or the user does not own it."""

class EnergyError(Exception):
    """Base for user-facing failures; the CLI prints these without a traceback."""


class ExtensionsNotFound(EnergyError):
    pass


class MissingLoD(EnergyError):
    pass

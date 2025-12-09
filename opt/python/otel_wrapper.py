import os
from importlib import import_module
import lumigo_opentelemetry
from opentelemetry.instrumentation.aws_lambda import AwsLambdaInstrumentor


def modify_module_name(module_name):
    """Returns a valid modified module to get imported"""
    return ".".join(module_name.split("/"))


class HandlerError(Exception):
    pass

path = os.environ.get("ORIG_HANDLER")

if path is None:
    raise HandlerError("ORIG_HANDLER is not defined.")

path = modify_module_name(path)
os.environ["ORIG_HANDLER"] = path

AwsLambdaInstrumentor().instrument()

try:
    (mod_name, handler_name) = path.rsplit(".", 1)
except ValueError as e:
    raise HandlerError("Bad path '{}' for ORIG_HANDLER: {}".format(path, str(e)))

handler_module = import_module(mod_name)
lambda_handler = getattr(handler_module, handler_name)
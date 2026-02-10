import logging

import pytest
from unittest.mock import Mock, patch

from dash0_opentelemetry.instrumentations.botocore.parsers import SqsParser, AwsParser


EMPTY_SQS_RESULT_1 = {}
EMPTY_SQS_RESULT_2 = {"Messages": []}
NON_EMPTY_SQS_RESULT = {"Messages": [{"MessageId": "1234", "Body": "test"}]}


@pytest.mark.parametrize(
    "env_var_value, operation, result, should_skip",
    [
        # Check that empty sqs polls are skipped
        ("true", "ReceiveMessage", EMPTY_SQS_RESULT_1, True),
        ("true", "ReceiveMessage", EMPTY_SQS_RESULT_2, True),
        # Check that non-empty polls are not skipped
        ("true", "ReceiveMessage", NON_EMPTY_SQS_RESULT, False),
        # Check that other operations are not skipped
        ("true", "DeleteMessage", EMPTY_SQS_RESULT_1, False),
        ("true", "DeleteMessageBatch", EMPTY_SQS_RESULT_1, False),
        ("true", "SendMessage", EMPTY_SQS_RESULT_1, False),
        ("true", "SendMessageBatch", EMPTY_SQS_RESULT_1, False),
        ("true", "UnknownOperation", EMPTY_SQS_RESULT_1, False),
        ("true", None, EMPTY_SQS_RESULT_1, False),
        # Check that empty sqs polls are not skipped if the env var is set to false
        ("false", "ReceiveMessage", EMPTY_SQS_RESULT_1, False),
        ("false", "ReceiveMessage", EMPTY_SQS_RESULT_2, False),
        # Check that non-empty polls are not skipped if the env var is set to false
        ("false", "ReceiveMessage", NON_EMPTY_SQS_RESULT, False),
        # Check that the default behavior is to skip empty sqs polls
        (None, "ReceiveMessage", EMPTY_SQS_RESULT_1, True),
        (None, "ReceiveMessage", EMPTY_SQS_RESULT_2, True),
        ("UnsupportedEnvVarValue", "ReceiveMessage", EMPTY_SQS_RESULT_2, True),
    ],
)
def test_sqs_skip_sqs_response(
    env_var_value, operation, result, should_skip, monkeypatch
):
    if env_var_value is not None:
        monkeypatch.setenv("LUMIGO_AUTO_FILTER_EMPTY_SQS", env_var_value)

    assert (
        SqsParser._should_skip_empty_sqs_polling_response(operation, result)
        == should_skip
    )


@patch(
    "dash0_opentelemetry.instrumentations.botocore.parsers.SqsParser._should_skip_empty_sqs_polling_response"
)
def test_parse_sqs_response_handles_empty_result(should_skip_mock):
    should_skip_mock.return_value = False
    span = Mock(set_attribute=Mock())
    service_name = "sqs"
    operation_name = "ReceiveMessage"

    # In case of an authentication error
    result = None

    # Check that no error is raised
    SqsParser.parse_response(span, service_name, operation_name, result)


@patch("dash0_opentelemetry.instrumentations.botocore.parsers.dump_with_context")
def test_parse_response_handles_unparsable_payload(dump_with_context_mock, caplog):
    dump_with_context_mock.side_effect = Exception("Boom!")

    span = Mock(set_attribute=Mock())
    result = {"content": {}}

    # no exception
    assert (
        AwsParser.parse_response(
            span=span,
            service_name="service-name",
            operation_name="operation",
            result=result,
        )
        is None
    )

    assert "An exception occurred in while extracting" in caplog.text

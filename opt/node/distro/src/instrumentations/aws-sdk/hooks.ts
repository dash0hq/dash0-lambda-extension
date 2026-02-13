import type {
  AwsSdkRequestHookInformation,
  AwsSdkResponseHookInformation,
} from '@opentelemetry/instrumentation-aws-sdk';
import type { Span as MutableSpan } from '@opentelemetry/sdk-trace-base';
import { AwsParsedService } from '../../spans/types';
import { extractAttributesFromSqsResponse } from './attribute-extractors';
import { CommonUtils, ScrubContext } from '@lumigo/node-core';
import { getSpanAttributeMaxLength } from '../../utils';
import {
  SEMATTRS_MESSAGING_OPERATION,
  SEMATTRS_RPC_METHOD,
  SEMATTRS_RPC_SERVICE,
} from '@opentelemetry/semantic-conventions';

const SQS_PUBLISH_OPERATIONS = ['SendMessage', 'SendMessageBatch'];
const SQS_CONSUME_OPERATIONS = ['ReceiveMessage'];

export const preRequestHook = (span: MutableSpan, requestInfo: AwsSdkRequestHookInformation) => {

  const sqsOperation = span.attributes?.[SEMATTRS_RPC_METHOD] as string;

  if (SQS_PUBLISH_OPERATIONS.includes(sqsOperation)) {
    span.setAttribute('aws.queue.name', span.attributes['messaging.destination.name']);
    span.setAttribute(SEMATTRS_MESSAGING_OPERATION, sqsOperation);
    span.setAttribute(
      'messaging.publish.body',
      CommonUtils.payloadStringify(
        requestInfo.request.commandInput,
        ScrubContext.HTTP_REQUEST_BODY,
        getSpanAttributeMaxLength()
      )
    );
  }
};

export const responseHook = (span: MutableSpan, responseInfo: AwsSdkResponseHookInformation) => {
  const awsServiceIdentifier = (span.attributes?.[SEMATTRS_RPC_SERVICE] as string)?.toLowerCase();

  if (awsServiceIdentifier === AwsParsedService.SQS) {
    const sqsOperation = span.attributes?.[SEMATTRS_RPC_METHOD] as string;

    span.setAttributes(extractAttributesFromSqsResponse(responseInfo.response.data, span));

    if (SQS_CONSUME_OPERATIONS.includes(sqsOperation)) {
      span.setAttribute('aws.queue.name', span.attributes['messaging.destination.name']);
      span.setAttribute(SEMATTRS_MESSAGING_OPERATION, sqsOperation);
      span.setAttribute(
        'messaging.consume.body',
        CommonUtils.payloadStringify(
          responseInfo.response.data,
          ScrubContext.HTTP_RESPONSE_BODY,
          getSpanAttributeMaxLength()
        )
      );
    }
  }
};

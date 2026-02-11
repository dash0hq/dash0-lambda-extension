import { responseHook, preRequestHook } from './hooks';
import type {
  AwsSdkRequestHookInformation,
  AwsSdkResponseHookInformation,
} from '@opentelemetry/instrumentation-aws-sdk';
import { getSpanAttributeMaxLength } from '../../utils';
import { SpanKind } from '@opentelemetry/api';
import {BasicTracerProvider, Span} from "@opentelemetry/sdk-trace-base";

export const rootSpanWithAttributes = (attributes: Record<string, any>, kind?: SpanKind): Span => {
  const provider = new BasicTracerProvider();
  const root = provider.getTracer('default').startSpan('root', { kind, attributes });
  root.setAttributes(attributes);

  return root as Span;
};

describe('aws-sdk instrumentation hooks', () => {
  describe('responseHook', () => {
    test('adds custom attributes to an SQS.ReceiveMessage span', () => {
      const span = rootSpanWithAttributes({
        'rpc.service': 'SQS',
        'rpc.method': 'ReceiveMessage',
        'messaging.destination.name': 'some-queue-name',
      });
      const awsSdkResponse: AwsSdkResponseHookInformation = awsResponseWithData({
        Messages: [{ Body: 'something' }],
      });

      responseHook(span, awsSdkResponse);

      expect(span.attributes).toMatchObject({
          "messaging.destination.name": "some-queue-name",
          "rpc.method": "ReceiveMessage",
          "rpc.service": "SQS",
      });
      expect(span.attributes['SKIP_EXPORT']).toBeUndefined();
    });

    test('does not modify a non SQS.ReceiveMessage span', () => {
      const span = rootSpanWithAttributes({ 'rpc.service': 'SQS', 'rpc.method': 'SomeThingElse' });
      const awsSdkResponse: AwsSdkResponseHookInformation = awsResponseWithData({
        Messages: [{ Body: 'something' }],
      });

      responseHook(span, awsSdkResponse);

      expect(span.attributes['messaging.consume.body']).toBeUndefined();
      expect(span.attributes['SKIP_EXPORT']).toBeUndefined();
    });


    // test('truncates and scrubs the SQS message body for the ReceiveMessage operations', () => {
    //   const secretKey = 'shush';
    //   const secretValue = 'this is top secret';
    //
    //   // node-core loads the value of LUMIGO_SECRET_MASKING_REGEX_HTTP_RESPONSE_BODIES on require() time,
    //   // therefore we must use isolateModules and re-set its value so the change will take effect
    //   jest.isolateModules(() => {
    //     process.env['LUMIGO_SECRET_MASKING_REGEX_HTTP_RESPONSE_BODIES'] = JSON.stringify([
    //       `.*${secretKey}.*`,
    //     ]);
    //
    //     const span = rootSpanWithAttributes({
    //       'rpc.service': 'SQS',
    //       'rpc.method': 'ReceiveMessage',
    //     });
    //     const payload = {
    //       Messages: [{ Body: 'some message' }],
    //       [secretKey]: secretValue,
    //       'non-secret-key': 'a'.repeat(getSpanAttributeMaxLength() * 2),
    //     };
    //     const awsSdkResponse: AwsSdkResponseHookInformation = awsResponseWithData(payload);
    //
    //     const responseHook = jest.requireActual('./hooks').responseHook;
    //     responseHook(span, awsSdkResponse);
    //
    //     expect(span.attributes['messaging.consume.body']).not.toContain(secretValue);
    //     expect(span.attributes['messaging.consume.body']!.toString().length).toBeLessThanOrEqual(
    //       JSON.stringify(payload).length
    //     );
    //   });
    // });

    const awsResponseWithData = (data: unknown): AwsSdkResponseHookInformation => {
      return {
        response: {
          request: {
            commandInput: {},
            commandName: 'not used',
            serviceName: 'not used',
            region: 'us-west-2',
          },
          requestId: '1234',
          data,
        },
        moduleVersion: 'x.y.z',
      };
    };
  });

  describe('preRequestHook', () => {
    test.each(['SendMessage', 'SendMessageBatch'])(
      'adds attributes to a span coming from an SQS publish operation',
      (sqsOperation) => {
        const span = rootSpanWithAttributes({
          'rpc.service': 'SQS',
          'rpc.method': sqsOperation,
          'messaging.destination.name': 'some-queue-name',
        });
        const awsSdkRequest: AwsSdkRequestHookInformation = awsRequestWithCommandInput({
          some: 'thing',
        });

        preRequestHook(span, awsSdkRequest);

        expect(span.attributes).toMatchObject({
          'messaging.publish.body': JSON.stringify(awsSdkRequest.request.commandInput),
          'messaging.operation': sqsOperation,
          'aws.queue.name': 'some-queue-name',
        });
        expect(span.attributes['SKIP_EXPORT']).toBeUndefined();
      }
    );

    describe('scrubbing the request body', () => {
      const secretKey = 'shhhh';
      const secretValue = 'some-secret';

      test.each(['SendMessage', 'SendMessageBatch'])(
        'truncates and scrubs the SQS message body for %s operations',
        (sqsOperation) => {
          // node-core loads the value of LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES on require() time,
          // therefore we must use isolateModules and re-set its value so the change will take effect
          jest.isolateModules(() => {
            process.env['LUMIGO_SECRET_MASKING_REGEX_HTTP_REQUEST_BODIES'] = JSON.stringify([
              `.*${secretKey}.*`,
            ]);

            const span = rootSpanWithAttributes({
              'rpc.service': 'SQS',
              'rpc.method': sqsOperation,
            });
            const payload = {
              [secretKey]: secretValue,
              'non-secret-key': 'a'.repeat(getSpanAttributeMaxLength() * 2),
            };
            const awsSdkRequest: AwsSdkRequestHookInformation = awsRequestWithCommandInput(payload);
            const preRequestHook = jest.requireActual('./hooks').preRequestHook;
            preRequestHook(span, awsSdkRequest);

            expect(span.attributes['messaging.publish.body']).not.toContain(secretValue);
            expect(
              span.attributes['messaging.publish.body']!.toString().length
            ).toBeLessThanOrEqual(JSON.stringify(payload).length);
          });
        }
      );
    });

    const awsRequestWithCommandInput = (
      commandInput: Record<string, any>
    ): AwsSdkRequestHookInformation => {
      return {
        request: {
          commandInput,
          commandName: 'not used',
          serviceName: 'not used',
          region: 'not used',
        },
      };
    };
  });

});

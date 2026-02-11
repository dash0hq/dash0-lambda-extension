import { getAwsServiceData, getAwsServiceFromHost } from './awsSpan';
import type { Span } from '@opentelemetry/sdk-trace-base';
import { AwsOtherService, AwsParsedService } from './types';

describe('awsSpan', () => {
  describe('getAwsServiceFromHost', () => {
    test('with an ApiGateway', () => {
      expect(
        getAwsServiceFromHost(
          new URL('https://my_happy_api.execute-api.eu-central-1.amazonaws.com/production/')
            .hostname
        )
      ).toBe(AwsOtherService.ApiGateway);
    });

    test('with an SQS queue URL', () => {
      // Example from https://docs.aws.amazon.com/AWSSimpleQueueService/latest/APIReference/API_GetQueueUrl.html
      expect(
        getAwsServiceFromHost(
          new URL('https://sqs.us-east-1.amazonaws.com/177715257436/MyQueue').hostname
        )
      ).toBe(AwsParsedService.SQS);
    });
  });

  describe('getAwsServiceData', () => {
    describe('when native aws-sdk instrumentation is inapplicable', () => {
      test('do not raise when span attributes are undefined', () => {
        const requestData = {
          body: '',
          host: 'anything',
        };
        const awsServiceAttributes = getAwsServiceData(requestData, undefined, {} as Span);
        expect(awsServiceAttributes).toEqual({});
      });

    });
  });
});

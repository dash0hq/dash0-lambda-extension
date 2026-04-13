import { buildLayerArn } from '../src/layer-arn-builder';

describe('buildLayerArn', () => {
  it('constructs correct ARN for us-east-1', () => {
    expect(buildLayerArn('us-east-1', 'dash0-extension-node', 42)).toBe(
      'arn:aws:lambda:us-east-1:115813213817:layer:dash0-extension-node:42'
    );
  });

  it('constructs correct ARN for eu-west-1', () => {
    expect(buildLayerArn('eu-west-1', 'dash0-extension-python', 1)).toBe(
      'arn:aws:lambda:eu-west-1:115813213817:layer:dash0-extension-python:1'
    );
  });

  it('constructs correct ARN for java layer', () => {
    expect(buildLayerArn('ap-southeast-1', 'dash0-extension-java', 100)).toBe(
      'arn:aws:lambda:ap-southeast-1:115813213817:layer:dash0-extension-java:100'
    );
  });
});

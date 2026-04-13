import { buildLayerArn, buildLayerFullName } from '../src/layer-arn-builder';

describe('buildLayerArn', () => {
  it('constructs correct ARN with default account', () => {
    expect(buildLayerArn('us-east-1', 'dash0-extension-node', 42)).toBe(
      'arn:aws:lambda:us-east-1:115813213817:layer:dash0-extension-node:42'
    );
  });

  it('constructs correct ARN for eu-west-1', () => {
    expect(buildLayerArn('eu-west-1', 'dash0-extension-python', 1)).toBe(
      'arn:aws:lambda:eu-west-1:115813213817:layer:dash0-extension-python:1'
    );
  });

  it('uses custom account ID when provided', () => {
    expect(buildLayerArn('us-east-1', 'dash0-extension-node', 42, '999888777666')).toBe(
      'arn:aws:lambda:us-east-1:999888777666:layer:dash0-extension-node:42'
    );
  });
});

describe('buildLayerFullName', () => {
  it('constructs full layer name with default account', () => {
    expect(buildLayerFullName('us-east-1', 'dash0-extension-node')).toBe(
      'arn:aws:lambda:us-east-1:115813213817:layer:dash0-extension-node'
    );
  });

  it('uses custom account ID when provided', () => {
    expect(buildLayerFullName('us-east-1', 'dash0-extension-node', '999888777666')).toBe(
      'arn:aws:lambda:us-east-1:999888777666:layer:dash0-extension-node'
    );
  });
});

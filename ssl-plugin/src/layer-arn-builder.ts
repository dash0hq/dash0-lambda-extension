const DASH0_LAYER_ACCOUNT_ID = '115813213817';

export function buildLayerArn(region: string, layerName: string, layerVersion: number): string {
  return `arn:aws:lambda:${region}:${DASH0_LAYER_ACCOUNT_ID}:layer:${layerName}:${layerVersion}`;
}

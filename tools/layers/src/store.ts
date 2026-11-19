import {
  AddLayerVersionPermissionCommand,
  LambdaClient,
  ListLayerVersionsCommand,
  PublishLayerVersionCommand,
  ResourceConflictException,
  ResourceNotFoundException,
  type Runtime,
} from '@aws-sdk/client-lambda';

export interface LayerVersion {
  version: number;
  description: string;
}

export interface PublishRequest {
  zip: Uint8Array;
  description: string;
  compatibleRuntimes?: string[];
}

export interface Published {
  version: number;
  compatibleRuntimes: string[];
}

/** The Lambda layer operations of one region that publishing needs. */
export interface LayerStore {
  /** The newest version of the layer, or undefined if it has none. */
  latest(layer: string): Promise<LayerVersion | undefined>;
  /** Publishes a new version; Lambda assigns its number. */
  publish(layer: string, request: PublishRequest): Promise<Published>;
  /** Lets every AWS account use the version; false if it already could. */
  makePublic(layer: string, version: number): Promise<boolean>;
}

/** A LayerStore backed by the Lambda API of one region. */
export class AwsLayerStore implements LayerStore {
  private readonly client: LambdaClient;

  /** Without a region, uses the one the AWS SDK resolves, such as AWS_REGION. */
  constructor(region?: string) {
    this.client = new LambdaClient({ region, retryMode: 'adaptive', maxAttempts: 10 });
  }

  async latest(layer: string): Promise<LayerVersion | undefined> {
    try {
      // Lambda lists versions newest first.
      const { LayerVersions } = await this.client.send(
        new ListLayerVersionsCommand({ LayerName: layer, MaxItems: 1 }),
      );
      const newest = LayerVersions?.[0];
      if (newest?.Version === undefined) {
        return undefined;
      }
      return { version: newest.Version, description: newest.Description ?? '' };
    } catch (error) {
      if (error instanceof ResourceNotFoundException) {
        return undefined;
      }
      throw error;
    }
  }

  async publish(layer: string, request: PublishRequest): Promise<Published> {
    const response = await this.client.send(
      new PublishLayerVersionCommand({
        LayerName: layer,
        Description: request.description,
        Content: { ZipFile: request.zip },
        CompatibleRuntimes: request.compatibleRuntimes as Runtime[] | undefined,
        CompatibleArchitectures: request.compatibleRuntimes ? ['x86_64', 'arm64'] : undefined,
      }),
    );
    if (response.Version === undefined) {
      throw new Error(`${layer}: PublishLayerVersion returned no version`);
    }
    return { version: response.Version, compatibleRuntimes: response.CompatibleRuntimes ?? [] };
  }

  async makePublic(layer: string, version: number): Promise<boolean> {
    try {
      await this.client.send(
        new AddLayerVersionPermissionCommand({
          LayerName: layer,
          VersionNumber: version,
          StatementId: 'public',
          Action: 'lambda:GetLayerVersion',
          Principal: '*',
        }),
      );
      return true;
    } catch (error) {
      // The `public` statement is already there.
      if (error instanceof ResourceConflictException) {
        return false;
      }
      throw error;
    }
  }
}

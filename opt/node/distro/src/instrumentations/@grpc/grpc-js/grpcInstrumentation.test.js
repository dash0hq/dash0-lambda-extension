import Dash0GrpcInstrumentation from './GrpcInstrumentation';

describe('Dash0GrpcInstrumentation', () => {
  afterEach(() => {
    jest.clearAllMocks();
  });

  let dash0GrpcInstrumentation = new Dash0GrpcInstrumentation();

  test('getInstrumentedModules should return ["@grpc/grpc-js"]', () => {
    expect(dash0GrpcInstrumentation.getInstrumentedModules()).toEqual(['@grpc/grpc-js']);
  });
});

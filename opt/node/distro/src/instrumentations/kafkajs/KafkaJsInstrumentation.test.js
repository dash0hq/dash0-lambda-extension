import Dash0KafkaJsInstrumentation from './KafkaJsInstrumentation';

describe('Dash0KafkaJsInstrumentation', () => {
  afterEach(() => {
    jest.clearAllMocks();
  });

  let dash0KafkaJsInstrumentation = new Dash0KafkaJsInstrumentation();

  test('getInstrumentedModules should return ["kafkajs"]', () => {
    expect(dash0KafkaJsInstrumentation.getInstrumentedModules()).toEqual(['kafkajs']);
  });
});

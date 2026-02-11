import Dash0KafkaJsInstrumentation from './KafkaJsInstrumentation';

describe('Dash0KafkaJsInstrumentation', () => {
  afterEach(() => {
    jest.clearAllMocks();
  });

  let dash0KafkaJsInstrumentation = new Dash0KafkaJsInstrumentation();

  test('getInstrumentedModule should return "kafkajs"', () => {
    expect(dash0KafkaJsInstrumentation.getInstrumentedModule()).toEqual('kafkajs');
  });
});

import Dash0AmqplibInstrumentation from './AmqplibInstrumentation';

describe('Dash0AmqplibInstrumentation', () => {
  afterEach(() => {
    jest.clearAllMocks();
  });

  let dash0AmqplibInstrumentation = new Dash0AmqplibInstrumentation();

  test('getInstrumentedModule should return "amqplib"', () => {
    expect(dash0AmqplibInstrumentation.getInstrumentedModule()).toEqual('amqplib');
  });
});

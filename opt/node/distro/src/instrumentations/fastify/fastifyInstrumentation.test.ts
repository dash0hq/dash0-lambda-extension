import Dash0FastifyInstrumentation from './FastifyInstrumentation';

describe('Dash0FastifyInstrumentation', () => {
  let dash0FastifyInstrumentation = new Dash0FastifyInstrumentation();

  test('getInstrumentedModule should return "fastify"', () => {
    expect(dash0FastifyInstrumentation.getInstrumentedModule()).toEqual('fastify');
  });
});

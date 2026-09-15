import Dash0FastifyInstrumentation from './FastifyInstrumentation';

describe('Dash0FastifyInstrumentation', () => {
  let dash0FastifyInstrumentation = new Dash0FastifyInstrumentation();

  test('getInstrumentedModules should return ["fastify"]', () => {
    expect(dash0FastifyInstrumentation.getInstrumentedModules()).toEqual(['fastify']);
  });
});

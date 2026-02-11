import Dash0PrismaInstrumentation from './PrismaInstrumentation';

describe('Dash0PrismaInstrumentation', () => {
  afterEach(() => {
    jest.clearAllMocks();
  });

  let dash0PrismaInstrumentation = new Dash0PrismaInstrumentation();

  test('getInstrumentedModule should return "@prisma/client"', () => {
    expect(dash0PrismaInstrumentation.getInstrumentedModule()).toEqual('@prisma/client');
  });
});

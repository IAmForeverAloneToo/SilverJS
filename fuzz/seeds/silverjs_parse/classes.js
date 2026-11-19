class A extends B {
  #x = 1; static #y;
  constructor() { super(); }
  get x() { return this.#x; } set x(v) { #x in this; }
  static { this.#y = 0; }
  async *m() {}
}

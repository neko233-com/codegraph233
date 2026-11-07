package graph.fixture;

/** Owned static-analysis fixture; it is compiled but never executed. */
public final class GraphFixture {
    private static final long ID = 0x1020304050607080L;
    private static final double SCALE = 0.5;
    private GraphFixture() {}
    public static int pick(int value) { return value + 1; }
    public static int pick(String value) { return value.length(); }
    public static int choose() { return pick("x"); }
    public static Runnable task() { return () -> pick(1); }
    public static String resource() { return "analysis/猫😀.txt"; }
    public static long id() { return ID; }
    public static double scale() { return SCALE; }
    public static int dense(int n) {
        switch (n) { case 0: return 7; case 1: return 11; case 2: return 13; default: return -1; }
    }
    public static int sparse(int n) {
        switch (n) { case 1: return 3; case 10000: return 5; default: return -1; }
    }
}

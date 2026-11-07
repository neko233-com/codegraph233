import java.io.*;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import java.security.MessageDigest;
import java.util.*;
import java.util.zip.*;
import org.objectweb.asm.*;

/** Static class-file facts only. Never loads or executes an analyzed class. */
public final class BytecodeRelations {
    private static BufferedWriter output;
    private static int classes, methods, fields, calls, dynamicSites, failures;
    private static String entry;
    private static String hash(byte[] data) throws Exception {
        StringBuilder b = new StringBuilder();
        for (byte n : MessageDigest.getInstance("SHA-256").digest(data)) b.append(String.format("%02x", n & 255));
        return b.toString();
    }
    private static String quote(Object value) {
        if (value == null) return "null";
        if (value instanceof Number || value instanceof Boolean) return value.toString();
        StringBuilder b = new StringBuilder("\"");
        for (char c : value.toString().toCharArray()) {
            if (c == '\\' || c == '"') b.append('\\').append(c);
            else if (c < 32 || c > 126) b.append(String.format("\\u%04x", (int)c));
            else b.append(c);
        }
        return b.append('"').toString();
    }
    private static void emit(String kind, Object... values) {
        StringBuilder b = new StringBuilder("{\"kind\":").append(quote(kind)).append(",\"entry\":").append(quote(entry));
        for (int i = 0; i < values.length; i += 2) b.append(',').append(quote(values[i])).append(':').append(quote(values[i+1]));
        try { output.write(b.append("}\n").toString()); }
        catch (IOException error) { throw new UncheckedIOException(error); }
    }
    private static void handle(String kind, String caller, int site, Handle target) {
        emit(kind, "caller", caller, "site", site, "owner", target.getOwner(), "name", target.getName(),
             "descriptor", target.getDesc(), "tag", target.getTag(), "interface", target.isInterface());
    }
    public static void main(String[] args) throws Exception {
        if (args.length != 2) throw new IllegalArgumentException("Input ordinary JAR and new JSONL output required");
        Path input = Paths.get(args[0]), destination = Paths.get(args[1]);
        if (Files.size(input) > 512L * 1024 * 1024) throw new IOException("Archive exceeds 512 MiB bound");
        long total = 0;
        try (ZipFile jar = new ZipFile(input.toFile());
             BufferedWriter out = Files.newBufferedWriter(destination, StandardCharsets.UTF_8, StandardOpenOption.CREATE_NEW)) {
            output = out;
            entry = null;
            emit("manifest", "format", "codegraph233.jvm-facts", "version", 1,
                 "artifactSha256", hash(Files.readAllBytes(input)), "exporter", "BytecodeRelations/1");
            Enumeration<? extends ZipEntry> entries = jar.entries();
            while (entries.hasMoreElements()) {
                ZipEntry member = entries.nextElement();
                if (member.isDirectory() || !member.getName().endsWith(".class")) continue;
                entry = member.getName();
                if (++classes > 30000) throw new IOException("Class bound exceeded");
                byte[] data;
                try (InputStream stream = jar.getInputStream(member)) { data = stream.readNBytes(16 * 1024 * 1024 + 1); }
                if (data.length > 16 * 1024 * 1024 || (total += data.length) > 256 * 1024 * 1024) throw new IOException("Class byte bound exceeded");
                try {
                    ClassReader reader = new ClassReader(data);
                    final String owner = reader.getClassName();
                    emit("class", "name", owner, "major", reader.readUnsignedShort(6), "bytes", data.length,
                         "sha256", hash(data), "super", reader.getSuperName(), "access", reader.getAccess());
                    for (String face : reader.getInterfaces()) emit("implements", "owner", owner, "target", face);
                    reader.accept(new ClassVisitor(Opcodes.ASM9) {
                        public FieldVisitor visitField(int access, String name, String descriptor, String signature, Object value) {
                            ++fields; emit("field", "owner", owner, "name", name, "descriptor", descriptor, "access", access);
                            return null;
                        }
                        public MethodVisitor visitMethod(int access, String name, String descriptor, String signature, String[] exceptions) {
                            ++methods;
                            final String caller = owner + "#" + name + descriptor;
                            emit("method", "id", caller, "owner", owner, "name", name, "descriptor", descriptor, "access", access);
                            return new MethodVisitor(Opcodes.ASM9) {
                                int site, line;
                                public void visitLineNumber(int value, Label label) { line = value; }
                                public void visitMethodInsn(int opcode, String targetOwner, String targetName, String targetDescriptor, boolean face) {
                                    ++calls; emit("invoke", "caller", caller, "site", ++site, "sourceLine", line, "opcode", opcode,
                                        "owner", targetOwner, "name", targetName, "descriptor", targetDescriptor,
                                        "target", targetOwner + "#" + targetName + targetDescriptor, "interface", face);
                                }
                                public void visitInvokeDynamicInsn(String name, String descriptor, Handle bootstrap, Object... arguments) {
                                    ++dynamicSites;
                                    emit("invokedynamic", "caller", caller, "site", ++site, "sourceLine", line,
                                         "name", name, "descriptor", descriptor, "runtimeTargetResolved", false);
                                    handle("bootstrap", caller, site, bootstrap);
                                    for (Object argument : arguments) if (argument instanceof Handle) handle("bootstrap-handle", caller, site, (Handle)argument);
                                }
                                public void visitFieldInsn(int opcode, String targetOwner, String targetName, String targetDescriptor) {
                                    emit("field-access", "caller", caller, "site", ++site, "sourceLine", line, "opcode", opcode,
                                        "owner", targetOwner, "name", targetName, "descriptor", targetDescriptor);
                                }
                                public void visitLdcInsn(Object value) {
                                    if (value instanceof ConstantDynamic)
                                        throw new UnsupportedOperationException("ConstantDynamic requires the native CLASS/JAR reader; partial facts rejected");
                                    if (value instanceof String) {
                                        String literal = (String)value;
                                        if (literal.length() <= 4096) emit("string", "caller", caller, "site", ++site, "value", literal);
                                        else {
                                            try { emit("long-string", "caller", caller, "site", ++site, "length", literal.length(), "sha256", hash(literal.getBytes(StandardCharsets.UTF_8))); }
                                            catch (Exception error) { throw new IllegalStateException(error); }
                                        }
                                    } else if (value instanceof Handle) handle("constant-handle", caller, ++site, (Handle)value);
                                    else if (value instanceof Type) emit("type-constant", "caller", caller, "site", ++site, "descriptor", ((Type)value).getDescriptor());
                                }
                            };
                        }
                    }, ClassReader.SKIP_FRAMES);
                } catch (Exception error) { ++failures; emit("class-error", "errorType", error.getClass().getName(), "message", error.getMessage()); }
            }
            entry = null;
            emit("summary", "classes", classes, "methods", methods, "fields", fields, "directCalls", calls,
                 "invokedynamicSites", dynamicSites, "failedClasses", failures, "scope", "Static bytecode facts; dynamic targets and external method bodies unresolved");
        }
        if (failures != 0) throw new IOException("Some classes failed; retain partial facts as incomplete");
    }
}

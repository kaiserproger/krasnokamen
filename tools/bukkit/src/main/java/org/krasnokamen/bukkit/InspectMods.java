package org.krasnokamen.bukkit;

import java.io.BufferedWriter;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import java.util.jar.JarEntry;
import java.util.jar.JarFile;
import org.objectweb.asm.AnnotationVisitor;
import org.objectweb.asm.ClassReader;
import org.objectweb.asm.Handle;
import org.objectweb.asm.ClassVisitor;
import org.objectweb.asm.MethodVisitor;
import org.objectweb.asm.Opcodes;

/** Inspect mod annotations and bytecode references without loading classes. */
public final class InspectMods {
    private InspectMods() {}

    private record Reference(String owner, String name, String descriptor, String caller,
                             int opcode, boolean interfaceTarget) {}

    public static void main(String[] args) throws Exception {
        if (args.length != 2) {
            throw new IllegalArgumentException("Usage: InspectMods <input.jar> <output.tsv>");
        }

        Set<String> entries = new TreeSet<>();
        Map<Reference, Long> calls = new HashMap<>();
        Map<Reference, Long> fields = new HashMap<>();
        entries.add("inventory\tforge-static-v1");
        var digest = java.security.MessageDigest.getInstance("SHA-256");
        try (var input = new java.security.DigestInputStream(Files.newInputStream(Path.of(args[0])), digest)) {
            input.transferTo(java.io.OutputStream.nullOutputStream());
        }
        entries.add("input\t" + java.util.HexFormat.of().formatHex(digest.digest()));
        try (JarFile jar = new JarFile(args[0])) {
            for (JarEntry entry : Collections.list(jar.entries())) {
                if (entry.isDirectory() || !entry.getName().endsWith(".class")) continue;
                byte[] bytes;
                try (InputStream input = jar.getInputStream(entry)) {
                    bytes = input.readAllBytes();
                }
                try {
                    if (bytes.length < 4 || bytes[0] != (byte) 0xca || bytes[1] != (byte) 0xfe
                            || bytes[2] != (byte) 0xba || bytes[3] != (byte) 0xbe) {
                        throw new IllegalArgumentException("invalid class file magic");
                    }
                    new ClassReader(bytes).accept(new ClassVisitor(Opcodes.ASM9) {
                        private String className;

                        @Override
                        public void visit(int version, int access, String name, String signature,
                                          String superName, String[] interfaces) {
                            className = escape(name.replace('/', '.'));
                            entries.add("class\t" + className + "\t" + access + "\t"
                                    + escape(superName == null ? "" : superName) + "\t"
                                    + escape(String.join(",", interfaces)));
                        }

                        @Override
                        public AnnotationVisitor visitAnnotation(String descriptor, boolean visible) {
                            entries.add("class-annotation\t" + className + "\t" + escape(descriptor));
                            if (descriptor.equals("Lnet/minecraftforge/fml/common/Mod$EventBusSubscriber;")) {
                                return new AnnotationVisitor(Opcodes.ASM9) {
                                    private String modId = "";
                                    private String bus = "FORGE";
                                    private final Set<String> sides = new TreeSet<>(
                                            List.of("CLIENT", "DEDICATED_SERVER"));

                                    @Override
                                    public void visit(String name, Object value) {
                                        if ("modid".equals(name)) modId = escape((String) value);
                                        else throw new IllegalArgumentException("unknown subscriber value: " + name);
                                    }

                                    @Override
                                    public void visitEnum(String name, String type, String value) {
                                        if (!"bus".equals(name)
                                                || !type.equals("Lnet/minecraftforge/fml/common/Mod$EventBusSubscriber$Bus;")) {
                                            throw new IllegalArgumentException("invalid subscriber bus");
                                        }
                                        bus = escape(value);
                                    }

                                    @Override
                                    public AnnotationVisitor visitArray(String name) {
                                        if (!"value".equals(name)) {
                                            throw new IllegalArgumentException("unknown subscriber array: " + name);
                                        }
                                        sides.clear();
                                        return new AnnotationVisitor(Opcodes.ASM9) {
                                            @Override
                                            public void visitEnum(String key, String type, String value) {
                                                if (!type.equals("Lnet/minecraftforge/api/distmarker/Dist;")) {
                                                    throw new IllegalArgumentException("invalid subscriber distribution");
                                                }
                                                sides.add(escape(value));
                                            }
                                        };
                                    }

                                    @Override
                                    public void visitEnd() {
                                        entries.add("subscriber\t" + className + "\t" + modId
                                                + "\t" + bus + "\t" + String.join(",", sides));
                                    }
                                };
                            }
                            if (!descriptor.equals("Lnet/minecraftforge/fml/common/Mod;")
                                    && !descriptor.equals("Lnet/neoforged/fml/common/Mod;")) {
                                return null;
                            }
                            return new AnnotationVisitor(Opcodes.ASM9) {
                                private String modId;

                                @Override
                                public void visit(String name, Object value) {
                                    if ("value".equals(name)) {
                                        if (!(value instanceof String) || modId != null) {
                                            throw new IllegalArgumentException("invalid @Mod value");
                                        }
                                        modId = escape((String) value);
                                    }
                                }

                                @Override
                                public void visitEnd() {
                                    if (modId == null) {
                                        throw new IllegalArgumentException("missing @Mod value");
                                    }
                                    entries.add("entry\t" + modId + "\t" + className);
                                }
                            };
                        }

                        @Override
                        public org.objectweb.asm.FieldVisitor visitField(int access, String name,
                                String descriptor, String signature, Object value) {
                            entries.add("declared-field\t" + className + "\t" + escape(name)
                                    + "\t" + escape(descriptor) + "\t" + access);
                            return new org.objectweb.asm.FieldVisitor(Opcodes.ASM9) {
                                @Override
                                public AnnotationVisitor visitAnnotation(String annotation, boolean visible) {
                                    entries.add("field-annotation\t" + className + "\t" + escape(name)
                                            + "\t" + escape(descriptor) + "\t" + escape(annotation));
                                    return null;
                                }
                            };
                        }

                        @Override
                        public MethodVisitor visitMethod(int access, String name, String descriptor,
                                                         String signature, String[] exceptions) {
                            return new MethodVisitor(Opcodes.ASM9) {
                                private final String caller = className + "\t" + escape(name)
                                        + "\t" + escape(descriptor);
                                {
                                    entries.add("method\t" + caller + "\t" + access);
                                }
                                private boolean handler;
                                private boolean receiveCanceled;
                                private String priority = "NORMAL";

                                @Override
                                public AnnotationVisitor visitAnnotation(String annotation, boolean visible) {
                                    entries.add("method-annotation\t" + caller + "\t" + escape(annotation));
                                    if (!annotation.equals("Lnet/minecraftforge/eventbus/api/SubscribeEvent;")
                                            && !annotation.equals("Lnet/neoforged/bus/api/SubscribeEvent;")) {
                                        return null;
                                    }
                                    handler = true;
                                    return new AnnotationVisitor(Opcodes.ASM9) {
                                        @Override
                                        public void visit(String key, Object value) {
                                            if ("receiveCanceled".equals(key)) {
                                                receiveCanceled = (Boolean) value;
                                            } else {
                                                throw new IllegalArgumentException("unknown SubscribeEvent value: " + key);
                                            }
                                        }

                                        @Override
                                        public void visitEnum(String key, String type, String value) {
                                            String expected = annotation.replace("SubscribeEvent;", "EventPriority;");
                                            if (!"priority".equals(key) || !expected.equals(type)) {
                                                throw new IllegalArgumentException("invalid SubscribeEvent priority");
                                            }
                                            priority = escape(value);
                                        }
                                    };
                                }

                                @Override
                                public void visitEnd() {
                                    if (handler) {
                                        entries.add("handler\t" + caller + "\t" + access + "\t"
                                                + priority + "\t" + receiveCanceled);
                                    }
                                }

                                @Override
                                public void visitTypeInsn(int opcode, String type) {
                                    entries.add("type\t" + caller + "\t" + opcode + "\t" + escape(type));
                                }

                                @Override
                                public void visitLdcInsn(Object value) {
                                    if (value instanceof org.objectweb.asm.Type
                                            || value instanceof Handle
                                            || value instanceof org.objectweb.asm.ConstantDynamic) {
                                        entries.add("constant\t" + caller + "\t" + escape(value.toString()));
                                    }
                                }

                                @Override
                                public void visitInvokeDynamicInsn(String invokedName, String invokedDescriptor,
                                                                   Handle bootstrap, Object... arguments) {
                                    entries.add("dynamic\t" + caller + "\t" + escape(invokedName)
                                            + "\t" + escape(invokedDescriptor) + "\t" + escape(bootstrap.getOwner())
                                            + "\t" + escape(bootstrap.getName()) + "\t" + escape(bootstrap.getDesc()));
                                    for (Object argument : arguments) {
                                        if (argument instanceof Handle handle) {
                                            entries.add("dynamic-target\t" + caller + "\t"
                                                    + escape(handle.getOwner()) + "\t" + escape(handle.getName())
                                                    + "\t" + escape(handle.getDesc()) + "\t" + handle.getTag());
                                        }
                                    }
                                }

                                @Override
                                public void visitMethodInsn(int opcode, String owner, String name,
                                                            String descriptor, boolean isInterface) {
                                    count(calls, owner, name, descriptor, caller, opcode, isInterface);
                                }

                                @Override
                                public void visitFieldInsn(int opcode, String owner, String name,
                                                           String descriptor) {
                                    count(fields, owner, name, descriptor, caller, opcode, false);
                                }
                            };
                        }
                    }, ClassReader.SKIP_DEBUG | ClassReader.SKIP_FRAMES);
                } catch (RuntimeException exception) {
                    throw new IllegalArgumentException("Cannot inspect class " + entry.getName()
                            + ": " + exception.getMessage(), exception);
                }
            }
        }

        List<String> lines = new ArrayList<>(entries);
        addReferences(lines, "call", calls);
        addReferences(lines, "field", fields);
        Collections.sort(lines);
        try (BufferedWriter output = Files.newBufferedWriter(Path.of(args[1]), StandardCharsets.UTF_8)) {
            for (String line : lines) {
                output.write(line);
                output.write('\n');
            }
        }
    }

    private static void count(Map<Reference, Long> counts, String owner, String name, String descriptor,
                              String caller, int opcode, boolean interfaceTarget) {
        Reference reference = new Reference(escape(owner), escape(name), escape(descriptor),
                caller, opcode, interfaceTarget);
        counts.merge(reference, 1L, Math::addExact);
    }

    private static void addReferences(List<String> lines, String type, Map<Reference, Long> counts) {
        counts.forEach((reference, count) -> lines.add(type + "\t" + reference.owner() + "\t"
                + reference.name() + "\t" + reference.descriptor() + "\t" + reference.caller()
                + "\t" + reference.opcode() + "\t" + reference.interfaceTarget() + "\t" + count));
    }

    private static String escape(String value) {
        for (int index = 0; index < value.length(); index++) {
            if (Character.isISOControl(value.charAt(index))) {
                throw new IllegalArgumentException("TSV field contains a control character");
            }
        }
        return value.replace("\\", "\\\\");
    }
}

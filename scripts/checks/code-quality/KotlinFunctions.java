import com.intellij.openapi.util.Disposer;
import com.intellij.psi.PsiElement;
import com.intellij.psi.PsiErrorElement;
import com.intellij.psi.util.PsiTreeUtil;
import org.jetbrains.kotlin.cli.jvm.compiler.EnvironmentConfigFiles;
import org.jetbrains.kotlin.cli.jvm.compiler.KotlinCoreEnvironment;
import org.jetbrains.kotlin.config.CompilerConfiguration;
import org.jetbrains.kotlin.psi.*;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Arrays;
import java.util.Objects;
import java.util.stream.IntStream;

// Kotlin function metrics for scan.mjs. Reads absolute paths on stdin, one per
// line, and prints one JSON object per line: a function row, or
// {"file", "error"} for a file that did not parse.
class KotlinFunctions {
    static KtDeclarationWithBody declaration(PsiElement node) {
        if (node instanceof KtNamedFunction function && function.getName() != null) return function;
        if (node instanceof KtSecondaryConstructor constructor) return constructor;
        if (node instanceof KtPropertyAccessor accessor) return accessor;
        return null;
    }

    // `else if` continues its chain at the same depth, as in scan.mjs.
    static boolean isElseIf(PsiElement node) {
        return node instanceof KtIfExpression
                && node.getParent() instanceof KtContainerNodeForControlStructureBody container
                && container.getParent() instanceof KtIfExpression parent
                && parent.getElse() == node;
    }

    static void metrics(PsiElement node, int depth, int[] result) {
        if (declaration(node) != null) return;
        boolean control = node instanceof KtIfExpression || node instanceof KtLoopExpression || node instanceof KtCatchClause;
        if (control || node instanceof KtWhenEntry) result[0]++;
        if ((control && !isElseIf(node)) || node instanceof KtWhenExpression) depth++;
        result[1] = Math.max(depth, result[1]);
        for (var child : node.getChildren()) metrics(child, depth, result);
    }

    static int lineAt(int[] starts, int offset) {
        int index = Arrays.binarySearch(starts, offset);
        return index < 0 ? -index - 1 : index + 1;
    }

    static String json(String value) {
        var out = new StringBuilder("\"");
        for (char c : value.toCharArray()) {
            if (c == '"' || c == '\\') out.append('\\').append(c);
            else if (c < 0x20) out.append(String.format("\\u%04x", (int) c));
            else out.append(c);
        }
        return out.append('"').toString();
    }

    static void error(String path, String message) {
        System.out.printf("{\"file\":%s,\"error\":%s}%n", json(path), json(message));
    }

    static void scan(KtPsiFactory factory, String path) throws Exception {
        var filePath = Path.of(path);
        var source = Files.readString(filePath).replace("\r\n", "\n");
        var file = factory.createFile(filePath.getFileName().toString(), source);
        if (PsiTreeUtil.findChildOfType(file, PsiErrorElement.class) != null) {
            error(path, "Kotlin syntax error. Function metrics omitted for this file.");
            return;
        }
        var starts = IntStream.concat(IntStream.of(0), IntStream.range(0, source.length())
                .filter(index -> source.charAt(index) == '\n').map(index -> index + 1)).toArray();
        var rows = new StringBuilder();
        for (var node : PsiTreeUtil.collectElements(file, candidate -> declaration(candidate) != null)) {
            var callable = Objects.requireNonNull(declaration(node));
            var body = callable.getBodyExpression();
            if (body == null) continue;
            var name = callable.getName();
            if (callable instanceof KtSecondaryConstructor) name = "constructor";
            if (callable instanceof KtPropertyAccessor accessor) {
                name = accessor.getProperty().getName() + (accessor.isGetter() ? ".get" : ".set");
            }
            var result = new int[2];
            metrics(body, 0, result);
            int start = lineAt(starts, node.getTextRange().getStartOffset());
            int end = lineAt(starts, node.getTextRange().getEndOffset() - 1);
            rows.append(String.format(
                    "{\"file\":%s,\"name\":%s,\"line\":%d,\"endLine\":%d,\"parameters\":%d,\"branches\":%d,\"nesting\":%d}%n",
                    json(path), json(name), start, end, callable.getValueParameters().size(), result[0], result[1]));
        }
        System.out.print(rows);
    }

    public static void main(String[] args) throws Exception {
        var disposable = Disposer.newDisposable();
        try (var input = new BufferedReader(new InputStreamReader(System.in))) {
            var environment = KotlinCoreEnvironment.createForProduction(disposable, new CompilerConfiguration(),
                EnvironmentConfigFiles.JVM_CONFIG_FILES);
            var factory = new KtPsiFactory(environment.getProject());
            for (String path; (path = input.readLine()) != null;) {
                try {
                    scan(factory, path);
                } catch (Exception failure) { // one unreadable file must not drop the others
                    error(path, failure.getClass().getSimpleName() + ": " + failure.getMessage());
                }
            }
        } finally {
            Disposer.dispose(disposable);
        }
    }
}

package dev.solaris.agent.mcp;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;

/** Offline view of the same definitions that the Java MCP server registers. */
public final class LoaderSchema {
    public static void main(String[] args) {
        JsonArray tools = new JsonArray();
        for (McpToolDefinition tool : ClientMcpTools.definitions()) {
            tools.add(tool.toJson());
        }
        JsonObject result = new JsonObject();
        result.add("tools", tools);
        System.out.println(result);
    }
}

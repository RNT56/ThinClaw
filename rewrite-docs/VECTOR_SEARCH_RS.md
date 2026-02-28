# How the Orchestrator Queries the Database (Without an LLM)

A very common question when learning about AI Agent architecture is: _"If the Generative LLM doesn't have access to the database, how does the Rust Orchestrator know what to search for?"_

The answer lies in understanding the difference between a **Generative LLM** (like GPT-4, Claude, or Llama) and an **Embedding Model** (like `nomic-embed-text`).

The Rust Orchestrator uses math—specifically, **Vector Similarity Search**—to find the right information, without ever "thinking" about it or generating text.

---

## 1. The Two Types of AI Models

To understand this, we need to split the term "AI" into two distinct tools:

### Tool A: The Embedding Model (The Librarian)

- **Examples:** `nomic-embed-text`, `bge-small-en`, OpenAI's `text-embedding-3-small`.
- **What it does:** It takes a sentence and turns it into a list of numbers (a vector).
- **Cost/Size:** Microscopic. It takes milliseconds to run locally and requires very little RAM. It **cannot** generate text or answer questions. It only outputs numbers.

### Tool B: The Generative LLM (The Writer)

- **Examples:** GPT-4o, Claude 3.5 Sonnet, `llama-3-8b`.
- **What it does:** It takes a huge block of text (the prompt) and generates a human-like response.
- **Cost/Size:** Massive. It is slow, takes gigabytes of VRAM, and is the actual "brain" of the agent.

---

## 2. How Data is Saved to the Knowledge Base

Long before the user asks a question, the Rust Orchestrator is preparing the database.

1. **Ingestion:** You drop a PDF manual for your car into the ThinClaw UI.
2. **Chunking:** The Rust Orchestrator (using standard Rust string splitting, no AI) chops the massive PDF into 500-word paragraphs.
3. **Embedding:** The Rust Orchestrator passes each paragraph to the **Embedding Model** (Tool A).
4. **The Output:** The Embedding Model outputs an array of 768 numbers for each paragraph. These numbers represent the semantic "meaning" of the text.
   - _Example Array:_ `[0.012, -0.045, 0.892, ...]`
5. **Storage:** The Rust Orchestrator saves both the text paragraph AND the array of numbers into the `sqlite-vec` database.

**Notice:** The Generative LLM (Tool B) was never involved in this process.

---

## 3. How the Orchestrator Queries the Database

Now, you sit down at ThinClaw and type: _"What is the recommended tire pressure for my car?"_

Here is exactly how the Rust Orchestrator finds the answer without using the Generative LLM:

### Step 1: Embed the User's Question

The Rust Orchestrator takes your question and passes it to the tiny **Embedding Model** (Tool A).
The Embedding Model outputs an array of numbers representing the meaning of your question: `[0.010, -0.040, 0.880, ...]`.

### Step 2: Vector Math (Cosine Similarity)

The Rust Orchestrator takes that array of numbers and runs a SQL query on the `sqlite-vec` database.

It asks the database: _"Find the stored arrays of numbers that are mathematically closest to this new array of numbers."_

This uses a mathematical formula called **Cosine Similarity** or **Euclidean Distance**. The database instantly finds the top 3 paragraphs whose "meaning numbers" closely match the "meaning numbers" of your question.

- **Result 1 (95% Match):** "The optimal tire pressure for the 2024 model is 32 PSI cold."
- **Result 2 (80% Match):** "To change a flat tire, locate the spare in the trunk."
- **Result 3 (60% Match):** "Rotate your tires every 5,000 miles."

### Step 3: Waking up the Generative LLM

Now, the Rust Orchestrator finally wakes up the massive **Generative LLM** (Tool B) and gives it a highly specific text prompt:

> **System Prompt:** You are a helpful assistant. Use the following context to answer the user's question.
>
> **Context:**
>
> 1. The optimal tire pressure for the 2024 model is 32 PSI cold.
> 2. To change a flat tire, locate the spare in the trunk.
> 3. Rotate your tires every 5,000 miles.
>
> **User's Question:** What is the recommended tire pressure for my car?

### Step 4: The Answer

The Generative LLM reads the prompt and generates the final text:
_"According to the manual, the recommended tire pressure for your car is 32 PSI when the tires are cold."_

---

## Summary

The Rust Orchestrator doesn't need to "read and understand" the database. It uses a very cheap, fast mathematical trick (Embeddings + Vector Distance) to instantly find the relevant paragraphs, and _only then_ does it ask the expensive, powerful LLM to read those specific paragraphs and formulate an answer.

import { fireEvent, render, waitFor } from "@testing-library/react";
import { useDropzone } from "react-dropzone";
import { describe, expect, it, vi } from "vitest";

function ChatDropzone({
    onDrop,
}: {
    onDrop: (acceptedFiles: File[]) => void;
}) {
    const { getInputProps, getRootProps } = useDropzone({
        onDrop,
        noClick: true,
        accept: {
            "image/*": [],
            "application/pdf": [],
            "text/*": [],
        },
    });

    return (
        <div data-testid="dropzone" {...getRootProps()}>
            <input {...getInputProps()} />
        </div>
    );
}

function dataTransfer(files: File[]) {
    return {
        files,
        items: files.map((file) => ({
            kind: "file",
            type: file.type,
            getAsFile: () => file,
        })),
        types: ["Files"],
    };
}

describe("react-dropzone chat compatibility", () => {
    it("accepts the chat file types and rejects unsupported files", async () => {
        const onDrop = vi.fn();
        const image = new File(["image"], "photo.png", { type: "image/png" });
        const pdf = new File(["pdf"], "document.pdf", { type: "application/pdf" });
        const text = new File(["text"], "notes.txt", { type: "text/plain" });
        const archive = new File(["archive"], "bundle.zip", { type: "application/zip" });

        const { getByTestId } = render(<ChatDropzone onDrop={onDrop} />);

        fireEvent.drop(getByTestId("dropzone"), {
            dataTransfer: dataTransfer([image, pdf, text, archive]),
        });

        await waitFor(() => expect(onDrop).toHaveBeenCalledOnce());
        expect(onDrop.mock.calls[0]?.[0]).toEqual([image, pdf, text]);
        expect(onDrop.mock.calls[0]?.[1]).toEqual([
            expect.objectContaining({ file: archive }),
        ]);
    });
});

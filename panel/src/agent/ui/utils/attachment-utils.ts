// Adapted from Pi web UI 0.75.3 (MIT). See ../LICENSE.
export interface Attachment {
	id: string;
	type: "image" | "document";
	fileName: string;
	mimeType: string;
	size: number;
	content: string; // base64 encoded original data (without data URL prefix)
	extractedText?: string; // For documents: <pdf filename="..."><page number="1">text</page></pdf>
	preview?: string; // base64 image preview (first page for PDFs, or same as content for images)
}


// Image-only loading: no remote fetch, document parsing, or browser execution.
export async function loadAttachment(file: File | Blob, fileName?: string): Promise<Attachment> {
	if (!(file instanceof Blob) || !["image/png", "image/jpeg", "image/gif", "image/webp"].includes(file.type))
		throw new Error("仅支持 PNG、JPEG、GIF、WebP 图片");
	const content = await new Promise<string>((resolve, reject) => {
		const reader = new FileReader();
		reader.onload = () => resolve((reader.result as string).split(",", 2)[1]);
		reader.onerror = () => reject(reader.error || new Error("图片读取失败"));
		reader.readAsDataURL(file);
	});
	return { id: crypto.randomUUID(), type: "image", fileName: fileName || (file instanceof File ? file.name : "image"),
		mimeType: file.type, size: file.size, content, preview: content };
}
